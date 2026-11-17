//! Shared application state.
//!
//! One instance per process, handed to every transport by `Arc`. Policy and the
//! Discord client join it in later phases, so `stdio` and `http` never diverge
//! in what they enforce.

use crate::config::Config;
use crate::security::secrets::BotToken;

/// Process-wide state: validated configuration and the bot token.
#[derive(Debug)]
pub struct AppState {
    config: Config,
    token: BotToken,
}

impl AppState {
    /// Build state from validated configuration and a resolved bot token.
    ///
    /// Both are required: a process that cannot authenticate has no business
    /// starting, so the failure belongs in startup rather than in the first
    /// tool call.
    pub fn new(config: Config, token: BotToken) -> Self {
        Self { config, token }
    }

    /// The validated configuration.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// The bot token. Only the Discord adapter may read it.
    pub fn token(&self) -> &BotToken {
        &self.token
    }
}
