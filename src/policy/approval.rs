//! Approval for destructive actions.
//!
//! No trustworthy approval channel is available to the process: MCP has no
//! confirmation primitive the server can rely on, so an approval abstraction that
//! returns `Granted` would be theatre. The abstraction therefore ships with one
//! implementation, [`UnavailableApproval`], which denies everything.
//!
//! It is deliberately **not** wired into the tool path. Destructive and
//! administrative tools are bounded by their policy class, allowlist, rolling
//! budget and audit record instead; nothing in the gate claims a confirmation it
//! did not receive. The module remains the seam for a real approval channel.

use async_trait::async_trait;

use crate::error::AppError;
use crate::policy::audit::AuditTarget;
use crate::policy::rules::Policy;

/// The answer to an approval request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Approval {
    /// The action may proceed.
    Granted,
    /// The action must not proceed.
    Denied,
}

/// What an approval is being asked about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApprovalRequest<'a> {
    /// Operation name, stable across releases.
    pub action: &'a str,
    /// What the operation would affect.
    pub target: AuditTarget,
}

/// A source of approval decisions.
#[async_trait]
pub trait ApprovalSource: Send + Sync + std::fmt::Debug {
    /// Decide whether `request` may proceed.
    ///
    /// An error is treated as a denial by [`authorize_destructive`].
    async fn approve(&self, request: ApprovalRequest<'_>) -> Result<Approval, AppError>;
}

/// The only approval source in this build: it never grants.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnavailableApproval;

#[async_trait]
impl ApprovalSource for UnavailableApproval {
    async fn approve(&self, _request: ApprovalRequest<'_>) -> Result<Approval, AppError> {
        Ok(Approval::Denied)
    }
}

/// Decide whether a destructive action may run, failing closed.
///
/// Three ways to be refused, all of them `POLICY_BLOCKED`: confirmation was
/// disabled in configuration (which removes the control rather than removing the
/// need for it), the source denied, or the source failed.
pub async fn authorize_destructive(
    policy: &Policy,
    approvals: &dyn ApprovalSource,
    request: ApprovalRequest<'_>,
) -> Result<(), AppError> {
    if !policy.require_confirmation_for_destructive() {
        return Err(AppError::policy_blocked(
            "destructive actions always require confirmation in this build",
        ));
    }

    match approvals.approve(request).await? {
        Approval::Granted => Ok(()),
        Approval::Denied => Err(AppError::policy_blocked(format!(
            "`{}` was not approved",
            request.action
        ))),
    }
}
