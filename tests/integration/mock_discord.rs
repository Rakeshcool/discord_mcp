//! A minimal mock of the Discord REST API.
//!
//! The adapter reaches it through twilight's proxy hook, which rewrites the
//! request URL to `http://<host>/api/v10/<route>` — the same mechanism twilight
//! documents for its own HTTP proxy server. Nothing about the adapter is
//! special-cased for tests: it speaks to a real socket over plain HTTP.
//!
//! Deliberately hand-rolled on tokio rather than another dependency: this server
//! only has to serve GET requests with a JSON body and record what it saw.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// A request the mock saw.
#[derive(Debug, Clone)]
pub(crate) struct RecordedRequest {
    /// HTTP method.
    pub(crate) method: String,
    /// Request path, including the query string.
    pub(crate) path: String,
    /// Request body, as sent.
    pub(crate) body: String,
    headers: HashMap<String, String>,
}

impl RecordedRequest {
    /// A header value, case-insensitive.
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }

    /// A query parameter value.
    pub(crate) fn query(&self, key: &str) -> Option<String> {
        let (_, query) = self.path.split_once('?')?;
        query.split('&').find_map(|pair| {
            let (name, value) = pair.split_once('=')?;
            (name == key).then(|| value.to_string())
        })
    }

    /// The path without its query string.
    pub(crate) fn route(&self) -> &str {
        self.path.split('?').next().unwrap_or(&self.path)
    }

    /// The body parsed as JSON, which is what a `POST` sends.
    pub(crate) fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body)
            .unwrap_or_else(|error| panic!("body is not JSON ({error}): {:?}", self.body))
    }
}

/// A canned response.
#[derive(Debug, Clone)]
pub(crate) struct MockResponse {
    /// Status code.
    pub(crate) status: u16,
    /// Extra headers.
    pub(crate) headers: Vec<(String, String)>,
    /// Body.
    pub(crate) body: String,
    /// Declared `Content-Length`, when it should differ from the body sent.
    pub(crate) content_length: Option<usize>,
    /// Delay before responding, for timeout tests.
    pub(crate) delay: Duration,
}

impl MockResponse {
    /// A `200` response with a JSON body.
    pub(crate) fn json(body: impl Into<String>) -> Self {
        Self {
            status: 200,
            headers: Vec::new(),
            body: body.into(),
            content_length: None,
            delay: Duration::ZERO,
        }
    }

    /// A Discord-shaped error response.
    pub(crate) fn error(status: u16, code: u64, message: &str) -> Self {
        Self::json(format!("{{\"code\":{code},\"message\":\"{message}\"}}")).status(status)
    }

    /// Override the status code.
    pub(crate) fn status(mut self, status: u16) -> Self {
        self.status = status;
        self
    }

    /// Add a header.
    pub(crate) fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    /// Wait before responding.
    pub(crate) fn delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }

    /// Promise more bytes than are sent, then close the connection.
    ///
    /// This is what a connection dropped mid-body looks like to a client: the
    /// headers arrive, the body is short, and the socket ends.
    pub(crate) fn declare_length(mut self, length: usize) -> Self {
        self.content_length = Some(length);
        self
    }
}

/// A running mock server.
pub(crate) struct MockDiscord {
    base_url: String,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    _task: tokio::task::JoinHandle<()>,
}

impl MockDiscord {
    /// Bind an ephemeral loopback port and answer requests with `handler`.
    pub(crate) async fn start<F>(handler: F) -> Self
    where
        F: Fn(&RecordedRequest) -> MockResponse + Send + Sync + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind the mock Discord server");
        let address = listener.local_addr().expect("mock server address");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let handler = Arc::new(handler);

        let task = {
            let requests = Arc::clone(&requests);
            tokio::spawn(async move {
                loop {
                    let Ok((mut stream, _)) = listener.accept().await else {
                        break;
                    };
                    let handler = Arc::clone(&handler);
                    let requests = Arc::clone(&requests);
                    tokio::spawn(async move {
                        let Some(request) = read_request(&mut stream).await else {
                            return;
                        };
                        requests
                            .lock()
                            .expect("mock request log")
                            .push(request.clone());
                        let response = handler(&request);
                        if !response.delay.is_zero() {
                            tokio::time::sleep(response.delay).await;
                        }
                        let _ = write_response(&mut stream, response).await;
                    });
                }
            })
        };

        Self {
            base_url: format!("http://{address}/api"),
            requests,
            _task: task,
        }
    }

    /// The `discord.api_base` value that points at this server.
    pub(crate) fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Every request received so far, in arrival order.
    pub(crate) fn requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().expect("mock request log").clone()
    }

    /// How many requests have arrived.
    pub(crate) fn request_count(&self) -> usize {
        self.requests.lock().expect("mock request log").len()
    }
}

async fn read_request(stream: &mut TcpStream) -> Option<RecordedRequest> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    let headers_end = loop {
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(position) = find_headers_end(&buffer) {
            break position;
        }
    };

    let head = String::from_utf8_lossy(&buffer[..headers_end]).into_owned();
    let mut lines = head.lines();
    let request_line = lines.next()?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();

    let mut headers = HashMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }

    // A write carries a body, and the test asserts on it, so read exactly as many
    // bytes as the request declared.
    let length = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    while buffer.len() < headers_end + length {
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    let body_end = (headers_end + length).min(buffer.len());
    let body = String::from_utf8_lossy(&buffer[headers_end..body_end]).into_owned();

    Some(RecordedRequest {
        method,
        path,
        body,
        headers,
    })
}

fn find_headers_end(buffer: &[u8]) -> Option<usize> {
    buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| position + 4)
}

async fn write_response(stream: &mut TcpStream, response: MockResponse) -> std::io::Result<()> {
    let reason = match response.status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        _ => "Status",
    };
    let mut head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        response.status,
        reason,
        response.content_length.unwrap_or(response.body.len()),
    );
    for (name, value) in &response.headers {
        head.push_str(name);
        head.push_str(": ");
        head.push_str(value);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");

    stream.write_all(head.as_bytes()).await?;
    stream.write_all(response.body.as_bytes()).await?;
    stream.flush().await?;
    Ok(())
}
