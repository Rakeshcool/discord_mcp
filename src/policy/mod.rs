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
//! - [`approval`] — the fail-closed approval abstraction for destructive actions.
//! - [`audit`] — content-free records of what policy decided.
//!
//! Writes stay disabled by default and destructive tools do not exist, so no
//! code path here can delete or alter Discord data.

pub mod approval;
pub mod audit;
pub mod gate;
pub mod rules;

pub use approval::{
    Approval, ApprovalRequest, ApprovalSource, UnavailableApproval, authorize_destructive,
};
pub use audit::{AuditEvent, AuditOutcome, AuditSink, AuditTarget, LoggingAudit};
pub use gate::PolicyGate;
pub use rules::{Policy, SanitizedMessage, ToolRisk, classify_tool, sanitize_mentions};
