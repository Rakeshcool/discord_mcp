//! Stable error categories and their mapping.

use discord_mcp::error::{AppError, ConfigError, ErrorCategory};

#[test]
fn every_category_has_its_specification_code() {
    let expected = [
        (ErrorCategory::Configuration, "CONFIGURATION_ERROR"),
        (ErrorCategory::Authentication, "AUTHENTICATION_FAILED"),
        (ErrorCategory::Permission, "PERMISSION_DENIED"),
        (ErrorCategory::ResourceNotFound, "RESOURCE_NOT_FOUND"),
        (ErrorCategory::InvalidArgument, "INVALID_ARGUMENT"),
        (ErrorCategory::RateLimited, "RATE_LIMITED"),
        (ErrorCategory::DiscordApi, "DISCORD_API_ERROR"),
        (ErrorCategory::Network, "NETWORK_ERROR"),
        (ErrorCategory::Timeout, "TIMEOUT"),
        (ErrorCategory::PolicyBlocked, "POLICY_BLOCKED"),
        (ErrorCategory::Internal, "INTERNAL_ERROR"),
    ];

    assert_eq!(expected.len(), 11);
    for (category, code) in expected {
        assert_eq!(category.code(), code);
        assert_eq!(category.to_string(), code);
    }
    assert_eq!(ErrorCategory::ALL, expected.map(|(category, _)| category));
}

#[test]
fn only_transient_categories_are_retryable() {
    for category in ErrorCategory::ALL {
        let expected = matches!(
            category,
            ErrorCategory::RateLimited | ErrorCategory::Network | ErrorCategory::Timeout
        );
        assert_eq!(
            category.is_retryable(),
            expected,
            "unexpected retryability for {category}"
        );
    }
}

#[test]
fn display_prefixes_the_stable_code() {
    let error = AppError::policy_blocked("channel is not allowlisted");
    assert_eq!(
        error.to_string(),
        "POLICY_BLOCKED: channel is not allowlisted"
    );
    assert_eq!(error.code(), "POLICY_BLOCKED");
    assert_eq!(error.category(), ErrorCategory::PolicyBlocked);
    assert_eq!(error.message(), "channel is not allowlisted");
    assert!(!error.is_retryable());
}

#[test]
fn configuration_errors_become_configuration_failures() {
    let config_error = ConfigError::invalid("policy.channel_allowlist", "must not be empty");
    let error = AppError::from(config_error);

    assert_eq!(error.category(), ErrorCategory::Configuration);
    let message = error.to_string();
    assert!(message.starts_with("CONFIGURATION_ERROR: "), "{message}");
    assert!(message.contains("policy.channel_allowlist"), "{message}");
}
