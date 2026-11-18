//! Library for the Discord MCP server.
//!
//! Layering (each layer only calls the one below it):
//!
//! - [`transport`] — MCP transports (STDIO now, Streamable HTTP in a later phase). No Discord logic.
//! - [`mcp`] — MCP protocol layer: server handler, tool registry, resources, prompts.
//! - [`app`] — application services: startup orchestration and shared state.
//! - [`policy`] — allowlists, risk classification and mutation limits.
//! - [`discord`] — the Discord adapter. Discord models never leave this module.
//! - [`config`], [`security`], [`observability`], [`error`] — the foundation.
//!
//! The binary is a thin CLI around [`app::service::AppService`].

pub mod app;
pub mod config;
pub mod discord;
pub mod error;
pub mod mcp;
pub mod observability;
pub mod policy;
pub mod security;
pub mod transport;
