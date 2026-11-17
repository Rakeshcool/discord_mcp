//! Streamable HTTP transport.
//!
//! Layered deliberately, each layer failing closed on its own:
//!
//! 1. the bind address is loopback unless `http.allow_non_loopback` says
//!    otherwise (configuration validation refuses the combination without
//!    authentication and explicit origins, and this module re-checks the bind),
//! 2. a bearer token is required whenever `http.require_auth` is set, compared in
//!    constant time against the environment variable `http.auth_token_env`,
//! 3. `Host` must be this server's own authority, which is what stops DNS
//!    rebinding against a loopback listener,
//! 4. a present `Origin` must match `http.allowed_origins`; with an empty list any
//!    `Origin`-bearing request is refused, so a browser cannot drive this server
//!    by accident,
//! 5. a request must finish within `http.request_timeout_seconds`, and an idle
//!    session is closed after `http.session_expiry_seconds`,
//! 6. the MCP service itself caps request bodies and validates media types.
//!
//! Nothing is authenticated by `Host` or `Origin` alone — they are anti-CSRF and
//! anti-rebinding checks, and the bearer token is the authentication.

use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use rmcp::transport::streamable_http_server::StreamableHttpServerConfig;
use rmcp::transport::streamable_http_server::StreamableHttpService;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use tokio::net::TcpListener;
use tokio_stream::StreamExt as _;

use crate::app::state::AppState;
use crate::config::schema::{HttpConfig, is_loopback_host};
use crate::error::AppError;
use crate::mcp::server::DiscordMcpServer;
use crate::security::secrets::HttpToken;

/// Path the MCP endpoint is mounted on.
pub const MCP_PATH: &str = "/mcp";

/// Serve MCP over Streamable HTTP until the process stops.
pub async fn serve(state: Arc<AppState>) -> Result<(), AppError> {
    let config = state.config().http.clone();

    if !is_loopback_host(&config.host) && !config.allow_non_loopback {
        return Err(AppError::configuration(format!(
            "http.host `{}` is not a loopback address; set http.allow_non_loopback to bind it",
            config.host
        )));
    }

    // Resolve the bearer token before binding: a server that cannot authenticate
    // its callers must not start listening.
    let token = if config.require_auth {
        Some(Arc::new(HttpToken::from_env(&config.auth_token_env)?))
    } else {
        None
    };

    let mcp = StreamableHttpService::new(
        {
            let state = Arc::clone(&state);
            move || Ok(DiscordMcpServer::new(Arc::clone(&state)))
        },
        Arc::new(session_manager(&config)),
        service_config(&config),
    );

    let router = Router::new()
        .nest_service(MCP_PATH, mcp)
        .layer(middleware::from_fn_with_state(
            AuthGate {
                token: token.clone(),
            },
            require_bearer,
        ))
        // Outermost, so the deadline covers authentication and MCP handling too.
        .layer(middleware::from_fn_with_state(
            Limits {
                request: Duration::from_secs(config.request_timeout_seconds),
            },
            limit_request_time,
        ));

    let listener = TcpListener::bind((config.host.as_str(), config.port))
        .await
        .map_err(|error| {
            AppError::configuration(format!(
                "cannot bind http://{}:{}: {error}",
                config.host, config.port
            ))
        })?;
    let address = listener
        .local_addr()
        .map_err(|error| AppError::internal(format!("cannot read the bound address: {error}")))?;

    tracing::info!(
        address = %address,
        path = MCP_PATH,
        authentication = config.require_auth,
        origins = config.allowed_origins.len(),
        request_timeout_seconds = config.request_timeout_seconds,
        session_expiry_seconds = config.session_expiry_seconds,
        "MCP HTTP server listening"
    );

    axum::serve(listener, router)
        .await
        .map_err(|error| AppError::internal(format!("the HTTP transport stopped: {error}")))?;
    Ok(())
}

/// The session manager, with the configured idle expiry.
///
/// `SessionConfig::keep_alive` is the SDK's "close the session after this much
/// inactivity" timer, which is what `http.session_expiry_seconds` means. The SDK
/// default is 5 minutes; the configured value replaces it rather than being
/// layered on top.
fn session_manager(config: &HttpConfig) -> LocalSessionManager {
    let mut manager = LocalSessionManager::default();
    manager.session_config.keep_alive = Some(Duration::from_secs(config.session_expiry_seconds));
    manager
}

/// rmcp's service settings, derived from validated configuration.
fn service_config(config: &HttpConfig) -> StreamableHttpServerConfig {
    let mut settings = StreamableHttpServerConfig::default()
        // A plain JSON response for simple calls keeps clients that do not
        // implement SSE working; the service still streams when it must.
        .with_json_response(true)
        .with_allowed_hosts(allowed_hosts(config))
        .with_allowed_origins(config.allowed_origins.iter().cloned());
    settings.max_request_body_bytes = config.max_request_bytes;
    if settings.allowed_origins.is_empty() {
        // Empty list would otherwise mean "do not validate": any present Origin
        // must be refused instead.
        settings = settings.enforce_origin_validation();
    }
    settings
}

/// Authorities this server answers to, for `Host` validation.
///
/// Only the configured bind plus the loopback spellings of it are accepted, so a
/// DNS name that resolves to 127.0.0.1 cannot reach the endpoint.
fn allowed_hosts(config: &HttpConfig) -> Vec<String> {
    let host = config.host.trim();
    let port = config.port;
    let mut hosts = BTreeSet::new();

    hosts.insert(host.to_string());
    hosts.insert(format!("{host}:{port}"));

    if is_loopback_host(host) {
        for loopback in ["localhost", "127.0.0.1", "::1"] {
            hosts.insert(loopback.to_string());
            hosts.insert(format!("{loopback}:{port}"));
            hosts.insert(format!("[{loopback}]"));
            hosts.insert(format!("[{loopback}]:{port}"));
        }
    }

    hosts.into_iter().collect()
}

/// What the auth layer needs, cloned into every request.
#[derive(Clone)]
struct AuthGate {
    token: Option<Arc<HttpToken>>,
}

/// Require a bearer token when authentication is configured.
///
/// Deliberately blunt: a missing, malformed or wrong `Authorization` header all
/// produce the same `401`, so a caller learns nothing from the response.
async fn require_bearer(State(gate): State<AuthGate>, request: Request, next: Next) -> Response {
    let Some(token) = gate.token else {
        return next.run(request).await;
    };
    match bearer(request.headers()) {
        Some(presented) if token.matches(presented) => next.run(request).await,
        _ => unauthorized(),
    }
}

/// The token from an `Authorization: Bearer <token>` header.
fn bearer(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let token = token.trim();
    (!token.is_empty()).then_some(token)
}

fn unauthorized() -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::WWW_AUTHENTICATE,
        header::HeaderValue::from_static("Bearer realm=\"discord-mcp\""),
    );
    (
        StatusCode::UNAUTHORIZED,
        headers,
        "unauthorized: this endpoint requires a bearer token\n",
    )
        .into_response()
}

/// Per-request limits the HTTP layer enforces itself.
#[derive(Clone, Copy)]
struct Limits {
    request: Duration,
}

/// Bound how long one request may take, headers and response body together.
///
/// A timeout on the handler alone would be a placebo here: the MCP service
/// returns an SSE response as soon as it starts streaming, so a stalled tool
/// call would keep an "already answered" request alive indefinitely. The
/// deadline therefore covers the response body too and ends it with an error.
///
/// `GET` is exempt: that route is the session's long-lived standalone event
/// stream, and expiring it would disconnect every subscribed client.
async fn limit_request_time(
    State(limits): State<Limits>,
    request: Request,
    next: Next,
) -> Response {
    if request.method() == Method::GET {
        return next.run(request).await;
    }

    let deadline = tokio::time::Instant::now() + limits.request;
    let response = match tokio::time::timeout_at(deadline, next.run(request)).await {
        Ok(response) => response,
        Err(_) => return gateway_timeout(),
    };

    let (parts, body) = response.into_parts();
    let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
    let body = Body::from_stream(body.into_data_stream().timeout(remaining).map(
        |item| -> Result<Bytes, BoxError> {
            match item {
                Ok(Ok(chunk)) => Ok(chunk),
                Ok(Err(error)) => Err(Box::new(error)),
                Err(_) => Err(Box::new(RequestTimedOut)),
            }
        },
    ));
    Response::from_parts(parts, body)
}

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Marker for a response body cut off by the request deadline.
#[derive(Debug)]
struct RequestTimedOut;

impl fmt::Display for RequestTimedOut {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("the request exceeded http.request_timeout_seconds")
    }
}

impl std::error::Error for RequestTimedOut {}

fn gateway_timeout() -> Response {
    (
        StatusCode::GATEWAY_TIMEOUT,
        "request timed out: http.request_timeout_seconds elapsed\n",
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::HttpConfig;
    use axum::http::HeaderValue;

    fn config(host: &str, port: u16) -> HttpConfig {
        HttpConfig {
            host: host.to_string(),
            port,
            ..HttpConfig::default()
        }
    }

    #[test]
    fn the_host_allowlist_covers_only_this_server() {
        let hosts = allowed_hosts(&config("127.0.0.1", 8787));

        for expected in [
            "127.0.0.1",
            "127.0.0.1:8787",
            "localhost",
            "localhost:8787",
            "[::1]:8787",
        ] {
            assert!(hosts.iter().any(|host| host == expected), "{expected}");
        }
        for unexpected in ["evil.example", "evil.example:8787", "0.0.0.0:8787"] {
            assert!(
                !hosts.iter().any(|host| host == unexpected),
                "{unexpected} must not be trusted"
            );
        }
    }

    #[test]
    fn a_non_loopback_bind_is_allowed_only_by_its_own_name() {
        let hosts = allowed_hosts(&config("192.0.2.10", 8787));

        assert_eq!(hosts, vec!["192.0.2.10", "192.0.2.10:8787"]);
    }

    #[test]
    fn empty_origins_are_enforced_rather_than_ignored() {
        let empty = service_config(&config("127.0.0.1", 8787));
        assert!(empty.allowed_origins.is_empty());

        let mut with_origin = config("127.0.0.1", 8787);
        with_origin.allowed_origins = vec!["https://app.example.com".to_string()];
        let settings = service_config(&with_origin);
        assert_eq!(
            settings.allowed_origins,
            vec!["https://app.example.com".to_string()]
        );
    }

    #[test]
    fn the_session_manager_uses_the_configured_expiry() {
        let mut configured = config("127.0.0.1", 8787);
        configured.session_expiry_seconds = 120;

        let manager = session_manager(&configured);
        assert_eq!(
            manager.session_config.keep_alive,
            Some(Duration::from_secs(120)),
            "the configured expiry must replace the SDK default, not be ignored"
        );
    }

    #[tokio::test]
    async fn a_response_body_that_outlives_the_deadline_is_cut_off() {
        use axum::routing::post;
        use tower::ServiceExt as _;

        async fn slow_body() -> Body {
            let chunks =
                tokio_stream::iter(vec![Bytes::from_static(b"late")]).then(|chunk| async move {
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    Ok::<_, std::io::Error>(chunk)
                });
            Body::from_stream(chunks)
        }

        let app =
            Router::new()
                .route("/slow", post(slow_body))
                .layer(middleware::from_fn_with_state(
                    Limits {
                        request: Duration::from_millis(50),
                    },
                    limit_request_time,
                ));

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/slow")
                    .body(Body::empty())
                    .expect("a request"),
            )
            .await
            .expect("the router answers");
        assert_eq!(response.status(), StatusCode::OK);

        let error = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .expect_err("a stalled body must not be served as a complete response");
        assert!(
            error.to_string().contains("request_timeout_seconds"),
            "unexpected error: {error}"
        );
    }

    #[tokio::test]
    async fn the_standalone_event_stream_is_not_cut_off() {
        use axum::routing::get;
        use tower::ServiceExt as _;

        async fn slow_stream() -> Body {
            let chunks =
                tokio_stream::iter(vec![Bytes::from_static(b"event")]).then(|chunk| async move {
                    tokio::time::sleep(Duration::from_millis(120)).await;
                    Ok::<_, std::io::Error>(chunk)
                });
            Body::from_stream(chunks)
        }

        let app =
            Router::new()
                .route("/events", get(slow_stream))
                .layer(middleware::from_fn_with_state(
                    Limits {
                        request: Duration::from_millis(20),
                    },
                    limit_request_time,
                ));

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/events")
                    .body(Body::empty())
                    .expect("a request"),
            )
            .await
            .expect("the router answers");
        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .expect("the event stream must outlive the request deadline");
        assert_eq!(body.as_ref(), b"event");
    }

    #[test]
    fn only_a_bearer_authorization_header_is_read() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer abc123"),
        );
        assert_eq!(bearer(&headers), Some("abc123"));

        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("bearer abc123"),
        );
        assert_eq!(bearer(&headers), Some("abc123"));

        for rejected in ["Basic abc123", "Bearer", "Bearer ", "abc123"] {
            headers.insert(
                header::AUTHORIZATION,
                HeaderValue::from_str(rejected).expect("a valid header value"),
            );
            assert_eq!(bearer(&headers), None, "{rejected} must not authenticate");
        }

        assert_eq!(bearer(&HeaderMap::new()), None);
    }
}
