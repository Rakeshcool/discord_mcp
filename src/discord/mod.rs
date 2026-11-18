//! Discord adapter.
//!
//! Everything Discord-specific lives here and nothing MCP-specific does:
//!
//! - [`ids`] — validated snowflake newtypes, so IDs stay strings on the wire and
//!   integers in memory,
//! - [`models`] — this crate's own guild, channel, message and member types,
//! - [`client`] — the application-owned [`DiscordApi`] trait,
//! - [`adapter`] — the twilight-http implementation behind that trait,
//! - [`errors`] — Discord failures mapped onto the stable error categories,
//! - [`rate_limit`] — rate-limit delegation and the bounded read retry policy.
//!
//! Only the official bot API is used: no user tokens, no self-bot paths, no
//! undocumented endpoints.

pub mod adapter;
pub mod client;
pub mod errors;
pub mod ids;
pub mod models;
pub mod permissions;
pub mod rate_limit;

pub use client::DiscordApi;
