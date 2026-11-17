//! Policy engine.
//!
//! MCP is the interface, not the permission system. This module will hold the
//! decisions that gate every tool call, applied in the application layer before
//! any Discord request:
//!
//! - feature flags (`read_enabled`, `write_enabled`),
//! - guild and channel allowlists,
//! - per-tool risk classification: read, write, destructive,
//! - limits for message reads, message length and concurrency,
//! - parsed-mention default-deny,
//! - the approval abstraction for destructive actions, which fails closed when
//!   no trustworthy approval flow exists.
