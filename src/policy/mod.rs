//! Policy engine.
//!
//! MCP is the interface, not the permission system. Decisions here are made in
//! the application layer from local configuration, before any Discord request is
//! built, and Discord message content can never influence them.
//!
//! - [`rules`] — feature flags, allowlists, risk classification, limits and
//!   mention neutralisation, as pure functions of configuration and the request.
//! - [`gate`] — the enforcing decorator that wraps the Discord client, so a tool
//!   cannot reach Discord without passing the rules.
//! - [`audit`] — content-free records of what policy decided.
//!
//! Every mutation class is disabled by default. When an operator enables one,
//! the gate applies its flag, allowlist, budget and audit path before a request
//! is built, and a dry run stops before all of them. There is no confirmation
//! prompt: `dry_run` is the plan step and the class flag plus allowlist is the
//! authority boundary.

pub mod audit;
pub mod gate;
pub mod rules;

pub use audit::{AuditEvent, AuditOutcome, AuditSink, AuditTarget, LoggingAudit};
pub use gate::PolicyGate;
pub use rules::{Policy, SanitizedMessage, ToolRisk, classify_tool, sanitize_mentions};
