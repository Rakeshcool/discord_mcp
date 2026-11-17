//! Approval for destructive actions: closed unless something trustworthy says
//! otherwise.

use async_trait::async_trait;

use discord_mcp::config::schema::PolicyConfig;
use discord_mcp::error::{AppError, ErrorCategory};
use discord_mcp::policy::approval::{
    Approval, ApprovalRequest, ApprovalSource, UnavailableApproval, authorize_destructive,
};
use discord_mcp::policy::audit::AuditTarget;
use discord_mcp::policy::rules::Policy;

fn policy(config: PolicyConfig) -> Policy {
    Policy::from_config(&config).expect("the test configuration must produce a policy")
}

fn request() -> ApprovalRequest<'static> {
    ApprovalRequest {
        action: "discord_delete_message",
        target: AuditTarget::none(),
    }
}

#[derive(Debug)]
struct Granting;

#[async_trait]
impl ApprovalSource for Granting {
    async fn approve(&self, _request: ApprovalRequest<'_>) -> Result<Approval, AppError> {
        Ok(Approval::Granted)
    }
}

#[derive(Debug)]
struct Failing;

#[async_trait]
impl ApprovalSource for Failing {
    async fn approve(&self, _request: ApprovalRequest<'_>) -> Result<Approval, AppError> {
        Err(AppError::internal("no approval channel is reachable"))
    }
}

#[tokio::test]
async fn the_shipped_approval_source_never_grants() {
    let policy = policy(PolicyConfig::default());

    let error = authorize_destructive(&policy, &UnavailableApproval, request())
        .await
        .expect_err("nothing can approve a destructive action in this build");

    assert_eq!(error.category(), ErrorCategory::PolicyBlocked);
}

#[tokio::test]
async fn a_granting_source_only_works_while_confirmation_is_required() {
    let required = policy(PolicyConfig {
        require_confirmation_for_destructive: true,
        ..PolicyConfig::default()
    });
    assert!(
        authorize_destructive(&required, &Granting, request())
            .await
            .is_ok()
    );

    let disabled = policy(PolicyConfig {
        require_confirmation_for_destructive: false,
        ..PolicyConfig::default()
    });
    let error = authorize_destructive(&disabled, &Granting, request())
        .await
        .expect_err("disabling confirmation must not remove the requirement");
    assert_eq!(error.category(), ErrorCategory::PolicyBlocked);
}

#[tokio::test]
async fn a_failing_approval_source_surfaces_its_error_and_grants_nothing() {
    let policy = policy(PolicyConfig::default());

    let error = authorize_destructive(&policy, &Failing, request())
        .await
        .expect_err("an unreachable approval channel cannot approve");

    assert_eq!(error.category(), ErrorCategory::Internal);
}
