//! A tiny blocking HTTP/1.1 client for the transport tests.
//!
//! Deliberately hand-rolled: the tests only need to send a JSON-RPC POST with a
//! chosen `Host`/`Origin`/`Authorization` and read a small response, and adding an
//! HTTP client dependency to a crate that rejects unnecessary ones would be worse
//! than fifty lines of `TcpStream`.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

/// A response, with headers kept case-insensitively reachable.
#[derive(Debug)]
pub(crate) struct HttpResponse {
    pub(crate) status: u16,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: String,
}

impl HttpResponse {
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// POST a JSON body to `path` on `address` (`host:port`).
pub(crate) fn post(
    address: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> HttpResponse {
    let host = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("host"))
        .map(|(_, value)| *value)
        .unwrap_or(address);

    let mut stream = TcpStream::connect(address).expect("connect to the test server");
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .expect("read timeout");

    let mut request = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("host") {
            continue;
        }
        request.push_str(name);
        request.push_str(": ");
        request.push_str(value);
        request.push_str("\r\n");
    }
    request.push_str("\r\n");
    request.push_str(body);

    stream
        .write_all(request.as_bytes())
        .expect("write the request");
    stream.flush().ok();

    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).expect("read the response");
    parse(&raw)
}

/// The `data:` payloads of a `text/event-stream` body, in order.
///
/// The transport streams an initialize response as SSE so a client can reconnect
/// and replay; the JSON-RPC message is the `data:` payload.
pub(crate) fn event_data(body: &str) -> Vec<&str> {
    body.lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(str::trim)
        .filter(|payload| !payload.is_empty())
        .collect()
}

fn parse(raw: &[u8]) -> HttpResponse {
    let split = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap_or_else(|| panic!("no header terminator in {:?}", String::from_utf8_lossy(raw)));
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let mut body = raw[split + 4..].to_vec();

    let mut lines = head.lines();
    let status_line = lines.next().expect("a status line");
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or_else(|| panic!("malformed status line {status_line:?}"));

    let mut headers = Vec::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_string(), value.trim().to_string()));
        }
    }

    let chunked = headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("transfer-encoding") && value.eq_ignore_ascii_case("chunked")
    });
    if chunked {
        body = dechunk(&body);
    }

    HttpResponse {
        status,
        headers,
        body: String::from_utf8_lossy(&body).into_owned(),
    }
}

fn dechunk(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(position) = rest.windows(2).position(|window| window == b"\r\n") {
        let size_line = String::from_utf8_lossy(&rest[..position]).into_owned();
        let size = usize::from_str_radix(size_line.trim().split(';').next().unwrap_or(""), 16)
            .unwrap_or(0);
        if size == 0 {
            break;
        }
        let start = position + 2;
        let end = (start + size).min(rest.len());
        out.extend_from_slice(&rest[start..end]);
        rest = &rest[(end + 2).min(rest.len())..];
    }
    out
}
