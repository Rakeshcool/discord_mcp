//! Discord adapter.
//!
//! This module owns everything Discord-specific and nothing MCP-specific.
//! Later phases add:
//!
//! - `client` — the `DiscordApi` trait the application layer talks to, plus the
//!   twilight-http implementation behind it,
//! - `models` — request and response types. They never leave this module; the
//!   application service sees only its own domain types,
//! - `errors` — mapping of Discord status codes onto
//!   [`crate::error::ErrorCategory`],
//! - `rate_limit` — bucket-aware waits and the bounded retry policy.
//!
//! Only the official bot API is used: no user tokens, no self-bot paths, no
//! undocumented endpoints.
