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
//! 5. the MCP service itself caps request bodies and validates media types.
//!
//! Nothing is authenticated by `Host` or `Origin` alone — they are anti-CSRF and
//! anti-rebinding checks, and the bearer token is the authentication.

use std::collections::BTreeSet;
use std::sync::Arc;

use axum::Router;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use rmcp::transport::streamable_http_server::StreamableHttpServerConfig;
use rmcp::transport::streamable_http_server::StreamableHttpService;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use tokio::net::TcpListener;

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
        Arc::new(LocalSessionManager::default()),
        service_config(&config),
    );

    let router = Router::new()
        .nest_service(MCP_PATH, mcp)
        .layer(middleware::from_fn_with_state(
            AuthGate {
                token: token.clone(),
            },
            require_bearer,
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
        "MCP HTTP server listening"
    );

    axum::serve(listener, router)
        .await
        .map_err(|error| AppError::internal(format!("the HTTP transport stopped: {error}")))?;
    Ok(())
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
