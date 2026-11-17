//! Shared application state.
//!
//! One instance per process, handed to every transport by `Arc`. Policy and
//! the Discord client live here too once their phases land, so `stdio` and
//! `http` never diverge in what they enforce.

use crate::config::Config;
use crate::error::AppError;
use crate::security::redaction::Redactor;
use crate::security::secrets::BotToken;

/// Process-wide state: validated configuration, the bot token and the redactor.
#[derive(Debug)]
pub struct AppState {
    config: Config,
    token: Option<BotToken>,
    redactor: Redactor,
}

impl AppState {
    /// Build state from validated configuration and an optional token.
    ///
    /// The token is optional so the process can validate configuration without
    /// credentials; `serve` requires one and fails closed without it.
    pub fn new(config: Config, token: Option<BotToken>) -> Self {
        Self {
            config,
            token,
            redactor: Redactor::new(),
        }
    }

    /// The validated configuration.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// The redactor used for logs and diagnostic output.
    pub fn redactor(&self) -> &Redactor {
        &self.redactor
    }

    /// The bot token, or an authentication error when none is available.
    pub fn token(&self) -> Result<&BotToken, AppError> {
        self.token.as_ref().ok_or_else(|| {
            AppError::configuration(format!(
                "no bot token is available; set the environment variable `{}`",
                self.config.discord.token_env
            ))
        })
    }
}
