//! STDIO transport.
//!
//! stdout carries MCP protocol frames and nothing else. Every diagnostic in the
//! process is routed to stderr by [`crate::observability::logging`], and the
//! crate denies `clippy::print_stdout`.
//!
//! Shutdown is driven by stdin closure, which is how an MCP client stops a
//! stdio server; Ctrl+C is handled by the process terminating.

use std::sync::Arc;

use rmcp::ServiceExt;
use rmcp::transport::stdio;

use crate::app::state::AppState;
use crate::error::AppError;
use crate::mcp::server::DiscordMcpServer;

/// Serve MCP over stdin/stdout until the peer closes the stream.
pub async fn serve(state: Arc<AppState>) -> Result<(), AppError> {
    let server = DiscordMcpServer::new(Arc::clone(&state));

    let service = server.serve(stdio()).await.map_err(|error| {
        AppError::internal(format!("failed to start the MCP stdio service: {error}"))
    })?;
    tracing::info!(transport = "stdio", "MCP server ready");

    let quit_reason = service.waiting().await.map_err(|error| {
        AppError::internal(format!("the MCP stdio service task failed: {error}"))
    })?;
    tracing::info!(
        transport = "stdio",
        reason = ?quit_reason,
        "MCP server stopped"
    );
    Ok(())
}
