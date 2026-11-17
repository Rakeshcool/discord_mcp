//! Stable error model.
//!
//! Every failure crossing the MCP boundary is reduced to an [`ErrorCategory`]
//! code from the specification plus a short, safe message. Messages are built
//! here, never copied from a lower layer verbatim, so a token, an authorization
//! header or a message body cannot ride along in an error string.

use std::fmt;

use thiserror::Error;

/// Stable, documented error categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorCategory {
    /// Configuration is missing, unparsable or internally inconsistent.
    Configuration,
    /// Discord rejected the bot credentials.
    Authentication,
    /// The bot or the local policy lacks permission for the operation.
    Permission,
    /// The guild, channel, message or member does not exist or is not visible.
    ResourceNotFound,
    /// A tool argument or identifier failed validation.
    InvalidArgument,
    /// Discord rate limited the request.
    RateLimited,
    /// Discord returned an error that maps to no narrower category.
    DiscordApi,
    /// The request never reached Discord, or the response was unreadable.
    Network,
    /// The request or the whole operation exceeded its deadline.
    Timeout,
    /// Local policy (allowlist, feature flag, limits) refused the operation.
    PolicyBlocked,
    /// A bug or an unexpected local failure. Details stay in redacted logs.
    Internal,
}

impl ErrorCategory {
    /// Every category, in specification order. Used by tests and diagnostics.
    pub const ALL: [ErrorCategory; 11] = [
        Self::Configuration,
        Self::Authentication,
        Self::Permission,
        Self::ResourceNotFound,
        Self::InvalidArgument,
        Self::RateLimited,
        Self::DiscordApi,
        Self::Network,
        Self::Timeout,
        Self::PolicyBlocked,
        Self::Internal,
    ];

    /// The stable wire code. Changing one of these strings is a breaking change.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Configuration => "CONFIGURATION_ERROR",
            Self::Authentication => "AUTHENTICATION_FAILED",
            Self::Permission => "PERMISSION_DENIED",
            Self::ResourceNotFound => "RESOURCE_NOT_FOUND",
            Self::InvalidArgument => "INVALID_ARGUMENT",
            Self::RateLimited => "RATE_LIMITED",
            Self::DiscordApi => "DISCORD_API_ERROR",
            Self::Network => "NETWORK_ERROR",
            Self::Timeout => "TIMEOUT",
            Self::PolicyBlocked => "POLICY_BLOCKED",
            Self::Internal => "INTERNAL_ERROR",
        }
    }

    /// Whether the same request may safely be tried again after a delay.
    ///
    /// Only reads and rate-limit waits are retryable. A write whose outcome is
    /// unknown is never retryable, whatever category it maps to.
    pub const fn is_retryable(self) -> bool {
        matches!(self, Self::RateLimited | Self::Network | Self::Timeout)
    }
}

impl fmt::Display for ErrorCategory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

/// An application error: a stable category plus a safe message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppError {
    category: ErrorCategory,
    message: String,
}

impl AppError {
    /// Build an error from an explicit category and message.
    ///
    /// The caller is responsible for keeping the message free of secrets,
    /// tokens, message content and raw stack traces.
    pub fn new(category: ErrorCategory, message: impl Into<String>) -> Self {
        Self {
            category,
            message: message.into(),
        }
    }

    /// A configuration or startup problem.
    pub fn configuration(message: impl Into<String>) -> Self {
        Self::new(ErrorCategory::Configuration, message)
    }

    /// Discord rejected the bot token.
    pub fn authentication(message: impl Into<String>) -> Self {
        Self::new(ErrorCategory::Authentication, message)
    }

    /// A caller-supplied argument failed validation.
    pub fn invalid_argument(message: impl Into<String>) -> Self {
        Self::new(ErrorCategory::InvalidArgument, message)
    }

    /// Local policy refused the operation.
    pub fn policy_blocked(message: impl Into<String>) -> Self {
        Self::new(ErrorCategory::PolicyBlocked, message)
    }

    /// An unexpected local failure.
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCategory::Internal, message)
    }

    /// The stable category of this error.
    pub const fn category(&self) -> ErrorCategory {
        self.category
    }

    /// The stable wire code of this error.
    pub const fn code(&self) -> &'static str {
        self.category.code()
    }

    /// The safe, human-readable message.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Whether the same request may safely be tried again.
    pub const fn is_retryable(&self) -> bool {
        self.category.is_retryable()
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.category, self.message)
    }
}

impl std::error::Error for AppError {}

/// A configuration or secret-resolution failure.
///
/// Separate from [`AppError`] so the configuration layer can stay free of
/// startup concerns; everything here converts to
/// [`ErrorCategory::Configuration`].
#[derive(Debug, Error)]
pub enum ConfigError {
    /// The configuration file could not be read.
    #[error("cannot read configuration file `{path}`: {source}")]
    Read {
        /// Path that was attempted.
        path: String,
        /// Underlying I/O failure.
        #[source]
        source: std::io::Error,
    },

    /// The configuration file is not valid TOML for this schema.
    #[error("invalid configuration in `{path}`: {message}")]
    Parse {
        /// Path that was parsed, or `built-in defaults`.
        path: String,
        /// Parser diagnostic. Never contains secret values.
        message: String,
    },

    /// A configuration value is present but unacceptable.
    #[error("invalid configuration: field `{field}` {reason}")]
    Invalid {
        /// Dotted field path, for example `policy.channel_allowlist`.
        field: &'static str,
        /// Why the value was rejected.
        reason: String,
    },

    /// A required secret is absent.
    #[error("required secret is missing: environment variable `{variable}` is not set")]
    MissingSecret {
        /// Environment variable that was consulted.
        variable: String,
    },

    /// A secret is present but obviously unusable.
    #[error("environment variable `{variable}` cannot be used as a bot token: {reason}")]
    InvalidSecret {
        /// Environment variable that was consulted.
        variable: String,
        /// Why the value was rejected. Never echoes the value itself.
        reason: String,
    },
}

impl ConfigError {
    /// Build an [`ConfigError::Invalid`] for a dotted field path.
    pub fn invalid(field: &'static str, reason: impl Into<String>) -> Self {
        Self::Invalid {
            field,
            reason: reason.into(),
        }
    }
}

impl From<ConfigError> for AppError {
    fn from(error: ConfigError) -> Self {
        Self::new(ErrorCategory::Configuration, error.to_string())
    }
}
