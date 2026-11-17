//! MCP transports.
//!
//! [`stdio`] is available now. The Streamable HTTP transport lands in a later
//! phase and will share this crate's application state, so both transports
//! enforce identical policy.

pub mod http;
pub mod stdio;
