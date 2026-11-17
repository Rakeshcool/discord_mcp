//! Mapping twilight failures onto the stable error categories.
//!
//! Two rules:
//!
//! 1. the category is chosen from Discord's status code, so a caller can act on
//!    it (`PERMISSION_DENIED` means fix permissions, `RATE_LIMITED` means wait);
//! 2. the message stays safe. Discord's error text describes the request, never
//!    the token, and it is truncated before it reaches a tool result.

use twilight_http::api_error::ApiError;
use twilight_http::error::ErrorType;

use crate::error::{AppError, ErrorCategory};

/// Longest Discord error text copied into a message.
const MAX_DETAIL_CHARS: usize = 200;

/// Text for a response whose body could not be read as the expected model.
const UNPARSABLE_RESPONSE: &str = "Discord returned a response that could not be parsed";

/// The error for a response whose body could not be read.
///
/// Shares its wording with [`ErrorType::Parsing`], because a caller cannot act
/// differently on "that JSON was not the shape I expected" depending on which
/// layer noticed.
pub fn read_body_error(operation: &str) -> AppError {
    AppError::new(
        ErrorCategory::Network,
        format!("{operation} failed: {UNPARSABLE_RESPONSE}"),
    )
}

/// Translate a twilight error into a category and safe message.
pub fn map_discord_error(operation: &str, error: &twilight_http::Error) -> AppError {
    let (category, detail) = classify(error.kind());
    match detail {
        Some(detail) => AppError::new(category, format!("{operation} failed: {detail}")),
        None => AppError::new(category, format!("{operation} failed")),
    }
}

fn classify(kind: &ErrorType) -> (ErrorCategory, Option<String>) {
    match kind {
        ErrorType::Unauthorized => (
            ErrorCategory::Authentication,
            Some("Discord rejected the bot token".to_string()),
        ),
        ErrorType::Response { status, error, .. } => {
            let category = match status.get() {
                401 => ErrorCategory::Authentication,
                403 => ErrorCategory::Permission,
                404 => ErrorCategory::ResourceNotFound,
                429 => ErrorCategory::RateLimited,
                400 | 405 | 422 => ErrorCategory::InvalidArgument,
                _ => ErrorCategory::DiscordApi,
            };
            let detail = api_error_text(error)
                .map(|text| format!("Discord returned {}: {text}", status.get()));
            (category, detail)
        }
        ErrorType::Parsing { .. } => (
            ErrorCategory::Network,
            Some(UNPARSABLE_RESPONSE.to_string()),
        ),
        ErrorType::RequestError => (
            ErrorCategory::Network,
            Some("the request to Discord did not complete".to_string()),
        ),
        ErrorType::RequestTimedOut => (
            ErrorCategory::Timeout,
            Some("the request to Discord timed out".to_string()),
        ),
        ErrorType::RequestCanceled => (
            ErrorCategory::Timeout,
            Some("the request to Discord was cancelled".to_string()),
        ),
        ErrorType::Validation => (
            ErrorCategory::InvalidArgument,
            Some("Discord rejected the request parameters".to_string()),
        ),
        ErrorType::BuildingRequest | ErrorType::CreatingHeader { .. } | ErrorType::Json => (
            ErrorCategory::Internal,
            Some("the Discord request could not be built".to_string()),
        ),
        _ => (
            ErrorCategory::Internal,
            Some("an unrecognised Discord client failure occurred".to_string()),
        ),
    }
}

/// Discord's own error text, truncated and stripped of anything unbounded.
fn api_error_text(error: &ApiError) -> Option<String> {
    let text = match error {
        ApiError::General(general) => format!("code {}: {}", general.code, general.message),
        ApiError::Ratelimited(limited) => {
            format!("rate limited, retry after {}s", limited.retry_after)
        }
        ApiError::Message(_) => "invalid message fields".to_string(),
        _ => return None,
    };
    Some(truncate(&text, MAX_DETAIL_CHARS))
}

fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut truncated: String = text.chars().take(max_chars).collect();
    truncated.push('…');
    truncated
}
