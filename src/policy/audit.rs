//! Audit events for policy decisions.
//!
//! An audit record answers "what was attempted, on which target, and what did
//! policy decide". It deliberately has no field that could hold message content,
//! a token or a tool argument, so a record cannot leak what it must not contain.
//!
//! The sink is injectable: this build logs records through `tracing`, and tests
//! collect them instead of scraping stderr.

use crate::discord::ids::{ChannelId, GuildId};
use crate::error::ErrorCategory;

/// What policy decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditOutcome {
    /// Policy allowed the operation to proceed.
    Allowed,
    /// Policy refused the operation.
    Denied,
    /// Policy allowed the operation and Discord rejected it.
    ///
    /// Writes produce exactly one record per attempt, so an attempt that gets
    /// past policy and then fails at Discord must not disappear from the trail.
    Failed,
}

impl AuditOutcome {
    /// Stable spelling for diagnostics.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::Denied => "denied",
            Self::Failed => "failed",
        }
    }
}

/// What an operation was aimed at. Identifiers only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AuditTarget {
    /// Guild the operation concerns, when known.
    pub guild: Option<GuildId>,
    /// Channel the operation concerns, when known.
    pub channel: Option<ChannelId>,
}

impl AuditTarget {
    /// No specific target, such as a guild listing.
    pub const fn none() -> Self {
        Self {
            guild: None,
            channel: None,
        }
    }

    /// A guild-scoped operation.
    pub const fn for_guild(guild: GuildId) -> Self {
        Self {
            guild: Some(guild),
            channel: None,
        }
    }

    /// A channel-scoped operation.
    pub const fn for_channel(channel: ChannelId) -> Self {
        Self {
            guild: None,
            channel: Some(channel),
        }
    }
}

/// One policy decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuditEvent {
    /// Operation name, stable across releases.
    pub action: &'static str,
    /// What policy decided.
    pub outcome: AuditOutcome,
    /// What the operation was aimed at.
    pub target: AuditTarget,
    /// Why policy refused, for denied and failed records.
    pub reason: Option<ErrorCategory>,
    /// The tool call this decision belongs to, when there is one.
    ///
    /// A tool call can produce several records (guild resolution, then the
    /// operation itself); the id is what ties them together without putting any
    /// content in the record.
    pub correlation: Option<u64>,
}

impl AuditEvent {
    /// A record for an allowed operation.
    pub const fn allowed(action: &'static str, target: AuditTarget) -> Self {
        Self {
            action,
            outcome: AuditOutcome::Allowed,
            target,
            reason: None,
            correlation: None,
        }
    }

    /// A record for a refused operation.
    pub const fn denied(action: &'static str, target: AuditTarget, reason: ErrorCategory) -> Self {
        Self {
            action,
            outcome: AuditOutcome::Denied,
            target,
            reason: Some(reason),
            correlation: None,
        }
    }

    /// A record for an attempt policy allowed that failed at Discord.
    pub const fn failed(action: &'static str, target: AuditTarget, reason: ErrorCategory) -> Self {
        Self {
            action,
            outcome: AuditOutcome::Failed,
            target,
            reason: Some(reason),
            correlation: None,
        }
    }

    /// Attach the tool call this decision belongs to.
    pub const fn with_correlation(mut self, correlation: Option<u64>) -> Self {
        self.correlation = correlation;
        self
    }
}

/// Where audit records go.
pub trait AuditSink: Send + Sync + std::fmt::Debug {
    /// Record one decision. Must not block or fail.
    fn record(&self, event: &AuditEvent);
}

/// The shipping sink: structured `tracing` records on stderr.
#[derive(Debug, Clone, Copy, Default)]
pub struct LoggingAudit;

impl AuditSink for LoggingAudit {
    fn record(&self, event: &AuditEvent) {
        // Allowances stay at debug so a busy read session is not noisy; a denial
        // is worth an operator's attention.
        match event.outcome {
            AuditOutcome::Allowed => tracing::debug!(
                action = event.action,
                outcome = event.outcome.as_str(),
                correlation = ?event.correlation,
                guild = ?event.target.guild,
                channel = ?event.target.channel,
                "policy allowed a Discord operation"
            ),
            AuditOutcome::Denied => tracing::info!(
                action = event.action,
                outcome = event.outcome.as_str(),
                correlation = ?event.correlation,
                guild = ?event.target.guild,
                channel = ?event.target.channel,
                code = ?event.reason.map(ErrorCategory::code),
                "policy denied a Discord operation"
            ),
            AuditOutcome::Failed => tracing::warn!(
                action = event.action,
                outcome = event.outcome.as_str(),
                correlation = ?event.correlation,
                guild = ?event.target.guild,
                channel = ?event.target.channel,
                code = ?event.reason.map(ErrorCategory::code),
                "an allowed Discord operation failed"
            ),
        }
    }
}
