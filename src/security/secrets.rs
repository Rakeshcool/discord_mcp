//! Bot token handling.
//!
//! The token is read once from an environment variable and wrapped in a
//! [`secrecy::SecretString`], which zeroizes on drop. It has no `Display`
//! implementation, its `Debug` output is redacted, and the only way to obtain
//! the value is [`BotToken::expose`], which the Discord adapter uses solely to
//! build an `Authorization` header.
//!
//! No user token, password or self-bot path exists in this crate.

use std::fmt;

use secrecy::{ExposeSecret, SecretString};

use crate::error::ConfigError;

/// Shortest value a Discord bot token can plausibly have.
const MIN_TOKEN_LEN: usize = 50;

/// Longest value a Discord bot token can plausibly have.
const MAX_TOKEN_LEN: usize = 400;

/// Substrings that indicate a placeholder rather than a real token.
const PLACEHOLDER_MARKERS: [&str; 5] = [
    "changeme",
    "your_token",
    "your-token",
    "replace_me",
    "replace-me",
];

/// A validated Discord bot token.
///
/// Shape validation is deliberately structural: it rejects values that cannot
/// be a token (empty, whitespace, placeholders, far too short) without pinning
/// Discord's internal token format, which is not a public contract.
pub struct BotToken(SecretString);

impl BotToken {
    /// Read the token from the environment variable `variable`.
    pub fn from_env(variable: &str) -> Result<Self, ConfigError> {
        Self::from_source(variable, std::env::var(variable).ok())
    }

    /// Validate a token that was already obtained, for example by a secret store.
    pub fn from_source(variable: &str, value: Option<String>) -> Result<Self, ConfigError> {
        let Some(raw) = value else {
            return Err(ConfigError::MissingSecret {
                variable: variable.to_string(),
            });
        };
        Self::validate(variable, raw)
    }

    /// The secret value. Only the Discord adapter may call this.
    pub fn expose(&self) -> &str {
        self.0.expose_secret()
    }

    fn validate(variable: &str, raw: String) -> Result<Self, ConfigError> {
        let reject = |reason: &str| ConfigError::InvalidSecret {
            variable: variable.to_string(),
            reason: reason.to_string(),
        };

        if raw.is_empty() {
            return Err(reject("it is empty"));
        }
        // Also covers surrounding whitespace and a pasted `Bot ` prefix.
        if raw.chars().any(char::is_whitespace) {
            return Err(reject(
                "it contains whitespace; paste the raw token without a `Bot ` prefix or surrounding spaces",
            ));
        }
        if raw.chars().any(char::is_control) {
            return Err(reject("it contains control characters"));
        }
        if raw.len() < MIN_TOKEN_LEN {
            return Err(reject("it is shorter than a Discord bot token"));
        }
        if raw.len() > MAX_TOKEN_LEN {
            return Err(reject("it is longer than a Discord bot token"));
        }
        if !raw.contains('.') {
            return Err(reject(
                "it does not look like a Discord bot token (missing `.` separators)",
            ));
        }
        let lowered = raw.to_ascii_lowercase();
        if PLACEHOLDER_MARKERS
            .iter()
            .any(|marker| lowered.contains(marker))
        {
            return Err(reject(
                "it looks like a placeholder value, not a real token",
            ));
        }

        Ok(Self(SecretString::from(raw)))
    }
}

impl fmt::Debug for BotToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BotToken(<redacted>)")
    }
}

/// Shortest bearer token the HTTP transport accepts.
const MIN_HTTP_TOKEN_LEN: usize = 16;

/// The bearer token guarding the Streamable HTTP transport.
///
/// Separate from [`BotToken`] because it is a different credential with a
/// different shape: this one is chosen by the operator, never sent to Discord,
/// and is compared against an inbound `Authorization` header.
///
/// The comparison is constant time, so a caller cannot learn the token one byte
/// at a time from response timings.
pub struct HttpToken(SecretString);

impl HttpToken {
    /// Read the token from the environment variable `variable`.
    pub fn from_env(variable: &str) -> Result<Self, ConfigError> {
        Self::from_source(variable, std::env::var(variable).ok())
    }

    /// Validate a token that was already obtained.
    pub fn from_source(variable: &str, value: Option<String>) -> Result<Self, ConfigError> {
        let Some(raw) = value else {
            return Err(ConfigError::MissingSecret {
                variable: variable.to_string(),
            });
        };
        Self::validate(variable, raw)
    }

    /// Whether `presented` is this token, compared in constant time.
    pub fn matches(&self, presented: &str) -> bool {
        constant_time_eq(self.0.expose_secret().as_bytes(), presented.as_bytes())
    }

    fn validate(variable: &str, raw: String) -> Result<Self, ConfigError> {
        let reject = |reason: &str| ConfigError::InvalidSecret {
            variable: variable.to_string(),
            reason: reason.to_string(),
        };

        if raw.is_empty() {
            return Err(reject("it is empty"));
        }
        if raw.chars().any(char::is_whitespace) {
            return Err(reject(
                "it contains whitespace; paste the token without spaces or a `Bearer ` prefix",
            ));
        }
        if raw.chars().any(char::is_control) {
            return Err(reject("it contains control characters"));
        }
        if raw.len() < MIN_HTTP_TOKEN_LEN {
            return Err(reject(
                "it is too short to be a useful bearer token (at least 16 characters)",
            ));
        }

        Ok(Self(SecretString::from(raw)))
    }
}

impl fmt::Debug for HttpToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("HttpToken(<redacted>)")
    }
}

/// Byte comparison whose duration does not depend on where the inputs differ.
fn constant_time_eq(expected: &[u8], presented: &[u8]) -> bool {
    if expected.len() != presented.len() {
        return false;
    }
    let mut difference = 0u8;
    for (left, right) in expected.iter().zip(presented) {
        difference |= left ^ right;
    }
    difference == 0
}
