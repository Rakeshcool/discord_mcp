//! Application service: the only place that starts the process.
//!
//! Startup order matters. The bot token is resolved first, so a missing secret
//! fails before any logging or transport work; logging is installed with
//! redaction before anything else can emit a record; only then is a transport
//! chosen.

use std::sync::Arc;

use crate::app::state::AppState;
use crate::config::{Config, TransportMode};
use crate::discord::adapter::TwilightDiscord;
use crate::error::AppError;
use crate::observability::logging;
use crate::policy::audit::LoggingAudit;
use crate::policy::gate::PolicyGate;
use crate::policy::rules::Policy;
use crate::security::secrets::BotToken;

/// Outcome of a configuration-only check, safe to print: no secret values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationReport {
    /// Whether the configured token variable is present in the environment.
    pub token_present: bool,
    /// Non-fatal observations an operator should read.
    pub warnings: Vec<String>,
}

/// The running application.
#[derive(Debug)]
pub struct AppService {
    state: Arc<AppState>,
}

impl AppService {
    /// Full startup: resolve the token, build the Discord client, install
    /// logging, then hand the transport a complete state.
    pub fn bootstrap(config: Config) -> Result<Self, AppError> {
        let token = BotToken::from_env(&config.discord.token_env)?;
        let policy = Policy::from_config(&config.policy)?;
        let discord = TwilightDiscord::new(&config.discord, &token)?;
        // Every Discord request goes through the gate: the raw client is not
        // stored anywhere a tool can reach it.
        let discord = PolicyGate::new(Arc::new(discord), policy, Arc::new(LoggingAudit));
        let state = Arc::new(AppState::new(config, token, Arc::new(discord)));
        logging::init(&state.config().logging)?;
        tracing::info!(
            server = %state.config().server.name,
            version = %state.config().server.version,
            transport = state.config().server.transport.as_str(),
            "starting"
        );
        Ok(Self { state })
    }

    /// Check configuration without serving and without requiring the token.
    pub fn validate_only(config: &Config) -> ValidationReport {
        let token_present = std::env::var(&config.discord.token_env)
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false);

        let mut warnings = Vec::new();
        if !token_present {
            warnings.push(format!(
                "`{}` is not set: `serve` will fail until it is",
                config.discord.token_env
            ));
        }
        if matches!(
            config.server.transport,
            TransportMode::Http | TransportMode::Both
        ) {
            warnings.push(
                "the Streamable HTTP transport is not implemented yet; only `stdio` serves in this build"
                    .to_string(),
            );
        }
        if config.policy.write_enabled {
            warnings.push(format!(
                "write tools are ENABLED for {} allowlisted channel(s)",
                config.policy.channel_allowlist.len()
            ));
        } else {
            warnings.push("write tools are disabled".to_string());
        }
        if !config.logging.redact_message_content {
            warnings.push(
                "message content logging is enabled; Discord message bodies will appear in logs"
                    .to_string(),
            );
        }

        ValidationReport {
            token_present,
            warnings,
        }
    }

    /// Run the configured transport until it stops.
    pub async fn run(self) -> Result<(), AppError> {
        match self.state.config().server.transport {
            TransportMode::Stdio => crate::transport::stdio::serve(Arc::clone(&self.state)).await,
            TransportMode::Http | TransportMode::Both => Err(AppError::configuration(
                "the Streamable HTTP transport is not implemented in this build; set `server.transport = \"stdio\"`",
            )),
        }
    }
}
