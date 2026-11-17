//! Shared application state.
//!
//! One instance per process, handed to every transport by `Arc`. Policy joins it
//! in the next phase, and both transports read from this same instance, so
//! `stdio` and `http` can never diverge in what they enforce.

use std::sync::Arc;

use crate::config::Config;
use crate::discord::DiscordApi;
use crate::security::secrets::BotToken;

/// Process-wide state: configuration, the bot token and the Discord client.
#[derive(Debug)]
pub struct AppState {
    config: Config,
    token: BotToken,
    discord: Arc<dyn DiscordApi>,
}

impl AppState {
    /// Build state from validated configuration, a resolved bot token and a
    /// Discord client.
    ///
    /// All three are required: a process that cannot authenticate has no
    /// business starting, so those failures belong in startup rather than in the
    /// first tool call.
    pub fn new(config: Config, token: BotToken, discord: Arc<dyn DiscordApi>) -> Self {
        Self {
            config,
            token,
            discord,
        }
    }

    /// The validated configuration.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// The bot token. Only the Discord adapter may read it.
    pub fn token(&self) -> &BotToken {
        &self.token
    }

    /// The Discord client used by every read operation.
    pub fn discord(&self) -> &Arc<dyn DiscordApi> {
        &self.discord
    }
}
