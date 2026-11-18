//! Typed configuration schema.
//!
//! Non-secret settings only. A secret never appears in this type: the
//! configuration names the environment variable that holds the bot token.
//!
//! [`Config::validate`] is the fail-closed gate. It rejects ambiguous or
//! unsafe combinations instead of guessing, and every rejection names the
//! offending field.

use serde::{Deserialize, Serialize};

use crate::error::ConfigError;

/// Accepted values for `logging.level`, matching tracing's level names.
pub const LOG_LEVELS: [&str; 5] = ["trace", "debug", "info", "warn", "error"];

/// Largest allowlist entry length: a Discord snowflake is at most 20 digits.
const MAX_SNOWFLAKE_LEN: usize = 20;

/// Which transports to run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TransportMode {
    /// MCP over stdin/stdout. The default.
    #[default]
    Stdio,
    /// MCP Streamable HTTP.
    Http,
    /// Both transports over shared state.
    Both,
}

impl TransportMode {
    /// The configuration spelling of this mode.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stdio => "stdio",
            Self::Http => "http",
            Self::Both => "both",
        }
    }
}

/// Top-level configuration document.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct Config {
    /// Server identity and transport selection.
    pub server: ServerConfig,
    /// Streamable HTTP settings.
    pub http: HttpConfig,
    /// Discord API and authentication settings.
    pub discord: DiscordConfig,
    /// Local policy: feature flags, allowlists and limits.
    pub policy: PolicyConfig,
    /// Logging and redaction settings.
    pub logging: LoggingConfig,
}

/// `[server]`
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct ServerConfig {
    /// Server name reported to MCP clients.
    pub name: String,
    /// Server version reported to MCP clients.
    pub version: String,
    /// Which transports to run.
    pub transport: TransportMode,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            name: "discord-mcp".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            transport: TransportMode::Stdio,
        }
    }
}

/// `[http]`
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct HttpConfig {
    /// Interface to bind. Loopback by default.
    pub host: String,
    /// TCP port to bind.
    pub port: u16,
    /// Require authentication for HTTP requests.
    pub require_auth: bool,
    /// Permit binding to a non-loopback interface. Off by default.
    pub allow_non_loopback: bool,
    /// Environment variable holding the HTTP bearer token.
    pub auth_token_env: String,
    /// Per-request timeout in seconds.
    pub request_timeout_seconds: u64,
    /// Maximum request body size in bytes.
    pub max_request_bytes: usize,
    /// Idle session expiry in seconds.
    pub session_expiry_seconds: u64,
    /// Non-`GET` requests the listener will handle at once.
    pub max_in_flight_requests: usize,
    /// Browser origins permitted by `Origin` validation.
    pub allowed_origins: Vec<String>,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 8787,
            require_auth: true,
            allow_non_loopback: false,
            auth_token_env: "DISCORD_MCP_HTTP_TOKEN".to_string(),
            request_timeout_seconds: 30,
            max_request_bytes: 1_048_576,
            session_expiry_seconds: 3_600,
            max_in_flight_requests: 64,
            allowed_origins: Vec::new(),
        }
    }
}

/// `[discord]`
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct DiscordConfig {
    /// Discord API base URL.
    pub api_base: String,
    /// Discord API version path segment. Pinned to `v10` for this release.
    pub api_version: String,
    /// Environment variable holding the bot token.
    pub token_env: String,
    /// Per-request timeout in seconds.
    pub request_timeout_seconds: u64,
    /// Maximum retries for retryable read failures.
    pub max_retries: u32,
    /// Permit a plaintext API base. Intended for loopback mock servers only.
    pub allow_insecure_transport: bool,
}

impl Default for DiscordConfig {
    fn default() -> Self {
        Self {
            api_base: "https://discord.com/api".to_string(),
            api_version: "v10".to_string(),
            token_env: "DISCORD_BOT_TOKEN".to_string(),
            request_timeout_seconds: 20,
            max_retries: 3,
            allow_insecure_transport: false,
        }
    }
}

/// `[policy]`
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct PolicyConfig {
    /// Enable read tools.
    pub read_enabled: bool,
    /// Enable write tools. Off by default.
    pub write_enabled: bool,
    /// Guilds the server may touch. Empty means every guild the bot can see.
    pub guild_allowlist: Vec<String>,
    /// Channels the server may touch. Required when writes are enabled.
    pub channel_allowlist: Vec<String>,
    /// Channels no tool may edit or delete, even when allowlisted.
    pub protected_channels: Vec<String>,
    /// Roles no tool may edit or delete, even when allowlisted.
    pub protected_roles: Vec<String>,
    /// Upper bound for one message read.
    pub max_messages_per_read: u16,
    /// Upper bound for an outgoing message body.
    pub max_message_characters: u16,
    /// Upper bound for the number of embeds on one send.
    pub max_embeds_per_message: u16,
    /// Upper bound for characters across every embed on one send.
    pub max_embed_characters: u16,
    /// Upper bound for uploaded files on one send.
    pub max_attachments_per_message: u16,
    /// Upper bound for one uploaded file, in bytes.
    pub max_attachment_bytes: u64,
    /// Upper bound for all uploaded files on one send, in bytes.
    pub max_attachment_total_bytes: u64,
    /// Directory that file uploads must stay beneath. No directory means file
    /// uploads are disabled.
    pub attachment_root: Option<String>,
    /// Whether parsed `@everyone`/`@here`, role and user mentions are allowed.
    pub allow_mass_mentions: bool,
    /// Upper bound for concurrent Discord requests.
    pub max_concurrent_requests: u16,
    /// Enable channel, thread and invite control tools.
    pub control_enabled: bool,
    /// Enable member nickname, kick, ban, unban and timeout tools.
    pub moderation_enabled: bool,
    /// Enable bulk delete and role tools.
    pub administrative_enabled: bool,
    /// Limits that apply when a tool mutates the server.
    pub autonomy: AutonomyConfig,
}

impl Default for PolicyConfig {
    fn default() -> Self {
        Self {
            read_enabled: true,
            write_enabled: false,
            guild_allowlist: Vec::new(),
            channel_allowlist: Vec::new(),
            protected_channels: Vec::new(),
            protected_roles: Vec::new(),
            max_messages_per_read: 100,
            max_message_characters: 2_000,
            max_embeds_per_message: 10,
            max_embed_characters: 6_000,
            max_attachments_per_message: 10,
            max_attachment_bytes: 8_388_608,
            max_attachment_total_bytes: 16_777_216,
            attachment_root: None,
            allow_mass_mentions: false,
            max_concurrent_requests: 8,
            control_enabled: false,
            moderation_enabled: false,
            administrative_enabled: false,
            autonomy: AutonomyConfig::default(),
        }
    }
}

/// `[policy.autonomy]`
///
/// The caps that make autonomous mutation tolerable. They are limits, not
/// suggestions: the policy gate enforces each one before a request is built.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct AutonomyConfig {
    /// Mutating actions allowed per minute, across every tool.
    pub max_actions_per_minute: u16,
    /// Messages `discord_send_message` may post to one channel per minute.
    pub max_sends_per_channel_per_minute: u16,
    /// Largest accepted bulk-delete batch.
    pub max_bulk_delete: u16,
    /// Tools that must be called with `dry_run = true`.
    pub require_dry_run: Vec<String>,
    /// File carrying the rolling budget across restarts. `None` uses the
    /// per-user default location.
    pub budget_state_file: Option<String>,
}

impl Default for AutonomyConfig {
    fn default() -> Self {
        Self {
            max_actions_per_minute: 30,
            max_sends_per_channel_per_minute: 5,
            max_bulk_delete: 50,
            require_dry_run: Vec::new(),
            budget_state_file: None,
        }
    }
}

/// `[logging]`
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct LoggingConfig {
    /// Minimum log level, one of [`LOG_LEVELS`].
    pub level: String,
    /// Never log message bodies when true.
    pub redact_message_content: bool,
    /// Log tool arguments when true. Off by default.
    pub log_tool_arguments: bool,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
            redact_message_content: true,
            log_tool_arguments: false,
        }
    }
}

impl Config {
    /// Validate the whole document, failing closed on the first problem.
    ///
    /// Rejections are deliberately strict: an ambiguous security setting is an
    /// error, not a warning with a guessed fallback.
    pub fn validate(&self) -> Result<(), ConfigError> {
        self.server.validate()?;
        self.http.validate()?;
        self.discord.validate()?;
        self.policy.validate()?;
        self.logging.validate()?;
        Ok(())
    }
}

impl ServerConfig {
    fn validate(&self) -> Result<(), ConfigError> {
        if self.name.trim().is_empty() {
            return Err(ConfigError::invalid("server.name", "must not be empty"));
        }
        if self.version.trim().is_empty() {
            return Err(ConfigError::invalid("server.version", "must not be empty"));
        }
        Ok(())
    }
}

impl HttpConfig {
    fn validate(&self) -> Result<(), ConfigError> {
        let host = self.host.trim();
        if host.is_empty() {
            return Err(ConfigError::invalid(
                "http.host",
                "must not be empty; use a loopback address such as 127.0.0.1",
            ));
        }
        if self.port == 0 {
            return Err(ConfigError::invalid(
                "http.port",
                "must be between 1 and 65535",
            ));
        }
        if !(1..=300).contains(&self.request_timeout_seconds) {
            return Err(ConfigError::invalid(
                "http.request_timeout_seconds",
                "must be between 1 and 300",
            ));
        }
        if !(1_024..=16_777_216).contains(&self.max_request_bytes) {
            return Err(ConfigError::invalid(
                "http.max_request_bytes",
                "must be between 1024 and 16777216",
            ));
        }
        if !(60..=86_400).contains(&self.session_expiry_seconds) {
            return Err(ConfigError::invalid(
                "http.session_expiry_seconds",
                "must be between 60 and 86400",
            ));
        }
        if !(1..=1_024).contains(&self.max_in_flight_requests) {
            return Err(ConfigError::invalid(
                "http.max_in_flight_requests",
                "must be between 1 and 1024",
            ));
        }
        if self.require_auth && !is_valid_env_name(self.auth_token_env.trim()) {
            return Err(ConfigError::invalid(
                "http.auth_token_env",
                "must name an environment variable when http.require_auth is true",
            ));
        }

        let loopback = is_loopback_host(host);
        if !loopback {
            if !self.allow_non_loopback {
                return Err(ConfigError::invalid(
                    "http.host",
                    format!(
                        "must be a loopback address unless http.allow_non_loopback is true (got `{host}`)"
                    ),
                ));
            }
            if !self.require_auth {
                return Err(ConfigError::invalid(
                    "http.require_auth",
                    "must be true when binding to a non-loopback address",
                ));
            }
            if self.allowed_origins.is_empty() {
                return Err(ConfigError::invalid(
                    "http.allowed_origins",
                    "must list the permitted browser origins when binding to a non-loopback address",
                ));
            }
        }

        for origin in &self.allowed_origins {
            let origin = origin.trim();
            if !(origin == "null"
                || origin.starts_with("https://")
                || origin.starts_with("http://"))
            {
                return Err(ConfigError::invalid(
                    "http.allowed_origins",
                    format!(
                        "entry `{origin}` must include a scheme (https://, http://) or be `null`"
                    ),
                ));
            }
        }

        Ok(())
    }
}

impl DiscordConfig {
    fn validate(&self) -> Result<(), ConfigError> {
        if self.api_version.trim() != "v10" {
            return Err(ConfigError::invalid(
                "discord.api_version",
                format!(
                    "must be `v10` (got `{}`); this release is tested against Discord API v10 only",
                    self.api_version.trim()
                ),
            ));
        }

        let base = self.api_base.trim();
        if base.is_empty() {
            return Err(ConfigError::invalid(
                "discord.api_base",
                "must not be empty",
            ));
        }
        if base.ends_with('/') {
            return Err(ConfigError::invalid(
                "discord.api_base",
                "must not end with `/`",
            ));
        }
        if let Some(rest) = base.strip_prefix("http://") {
            if !self.allow_insecure_transport {
                return Err(ConfigError::invalid(
                    "discord.api_base",
                    "must use https; set discord.allow_insecure_transport only when pointing at a loopback mock server",
                ));
            }
            let host = rest.split(['/', ':']).next().unwrap_or_default();
            if !is_loopback_host(host) {
                return Err(ConfigError::invalid(
                    "discord.api_base",
                    "may use plaintext http only for a loopback host",
                ));
            }
        } else if !base.starts_with("https://") {
            return Err(ConfigError::invalid(
                "discord.api_base",
                "must be an absolute http(s) URL",
            ));
        }

        if !is_valid_env_name(self.token_env.trim()) {
            return Err(ConfigError::invalid(
                "discord.token_env",
                "must name an environment variable, for example DISCORD_BOT_TOKEN",
            ));
        }
        if !(1..=120).contains(&self.request_timeout_seconds) {
            return Err(ConfigError::invalid(
                "discord.request_timeout_seconds",
                "must be between 1 and 120",
            ));
        }
        if self.max_retries > 5 {
            return Err(ConfigError::invalid(
                "discord.max_retries",
                "must be between 0 and 5",
            ));
        }
        Ok(())
    }
}

impl PolicyConfig {
    fn validate(&self) -> Result<(), ConfigError> {
        if self.allow_mass_mentions {
            return Err(ConfigError::invalid(
                "policy.allow_mass_mentions",
                "must remain false: parsed @everyone/@here, role and user mentions are not supported",
            ));
        }
        if self.write_enabled && self.channel_allowlist.is_empty() {
            return Err(ConfigError::invalid(
                "policy.channel_allowlist",
                "must list at least one channel ID when policy.write_enabled is true",
            ));
        }
        if self.control_enabled && self.channel_allowlist.is_empty() {
            return Err(ConfigError::invalid(
                "policy.channel_allowlist",
                "must list at least one channel ID when policy.control_enabled is true",
            ));
        }
        if self.moderation_enabled && self.guild_allowlist.is_empty() {
            return Err(ConfigError::invalid(
                "policy.guild_allowlist",
                "must list at least one guild ID when policy.moderation_enabled is true",
            ));
        }
        if self.administrative_enabled && self.guild_allowlist.is_empty() {
            return Err(ConfigError::invalid(
                "policy.guild_allowlist",
                "must list at least one guild ID when policy.administrative_enabled is true",
            ));
        }
        validate_allowlist("policy.guild_allowlist", &self.guild_allowlist)?;
        validate_allowlist("policy.channel_allowlist", &self.channel_allowlist)?;
        validate_allowlist("policy.protected_channels", &self.protected_channels)?;
        validate_allowlist("policy.protected_roles", &self.protected_roles)?;

        if !(1..=600).contains(&self.autonomy.max_actions_per_minute) {
            return Err(ConfigError::invalid(
                "policy.autonomy.max_actions_per_minute",
                "must be between 1 and 600",
            ));
        }
        if !(1..=600).contains(&self.autonomy.max_sends_per_channel_per_minute) {
            return Err(ConfigError::invalid(
                "policy.autonomy.max_sends_per_channel_per_minute",
                "must be between 1 and 600",
            ));
        }
        if let Some(path) = &self.autonomy.budget_state_file
            && path.trim().is_empty()
        {
            return Err(ConfigError::invalid(
                "policy.autonomy.budget_state_file",
                "must not be empty; omit it for the per-user default location",
            ));
        }
        if !(2..=100).contains(&self.autonomy.max_bulk_delete) {
            return Err(ConfigError::invalid(
                "policy.autonomy.max_bulk_delete",
                "must be between 2 and 100, which is Discord's own bulk-delete ceiling",
            ));
        }
        let mut dry_run_tools = std::collections::BTreeSet::new();
        for tool in &self.autonomy.require_dry_run {
            let tool = tool.trim();
            if !tool.starts_with("discord_") || tool.len() < 9 {
                return Err(ConfigError::invalid(
                    "policy.autonomy.require_dry_run",
                    format!("entry `{tool}` must name a tool, for example discord_delete_message"),
                ));
            }
            if !dry_run_tools.insert(tool.to_string()) {
                return Err(ConfigError::invalid(
                    "policy.autonomy.require_dry_run",
                    format!("entry `{tool}` is listed more than once"),
                ));
            }
        }

        if !(1..=100).contains(&self.max_messages_per_read) {
            return Err(ConfigError::invalid(
                "policy.max_messages_per_read",
                "must be between 1 and 100",
            ));
        }
        if !(1..=2_000).contains(&self.max_message_characters) {
            return Err(ConfigError::invalid(
                "policy.max_message_characters",
                "must be between 1 and 2000",
            ));
        }
        if !(0..=10).contains(&self.max_embeds_per_message) {
            return Err(ConfigError::invalid(
                "policy.max_embeds_per_message",
                "must be between 0 and 10, which is Discord's embed count ceiling",
            ));
        }
        if !(0..=6_000).contains(&self.max_embed_characters) {
            return Err(ConfigError::invalid(
                "policy.max_embed_characters",
                "must be between 0 and 6000, which is Discord's total embed character ceiling",
            ));
        }
        if !(0..=10).contains(&self.max_attachments_per_message) {
            return Err(ConfigError::invalid(
                "policy.max_attachments_per_message",
                "must be between 0 and 10, which is Discord's attachment count ceiling",
            ));
        }
        if !(1..=25_165_824).contains(&self.max_attachment_bytes) {
            return Err(ConfigError::invalid(
                "policy.max_attachment_bytes",
                "must be between 1 and 25165824 (25 MiB)",
            ));
        }
        if !(1..=104_857_600).contains(&self.max_attachment_total_bytes) {
            return Err(ConfigError::invalid(
                "policy.max_attachment_total_bytes",
                "must be between 1 and 104857600 (100 MiB)",
            ));
        }
        if let Some(root) = &self.attachment_root
            && root.trim().is_empty()
        {
            return Err(ConfigError::invalid(
                "policy.attachment_root",
                "must not be empty; omit it to disable file uploads",
            ));
        }
        if !(1..=64).contains(&self.max_concurrent_requests) {
            return Err(ConfigError::invalid(
                "policy.max_concurrent_requests",
                "must be between 1 and 64",
            ));
        }
        Ok(())
    }
}

impl LoggingConfig {
    fn validate(&self) -> Result<(), ConfigError> {
        let level = self.level.trim().to_ascii_lowercase();
        if !LOG_LEVELS.contains(&level.as_str()) {
            return Err(ConfigError::invalid(
                "logging.level",
                format!(
                    "must be one of {} (got `{}`)",
                    LOG_LEVELS.join(", "),
                    self.level.trim()
                ),
            ));
        }
        Ok(())
    }
}

/// Whether `host` only ever refers to the local machine.
pub fn is_loopback_host(host: &str) -> bool {
    let host = host.trim();
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.parse::<std::net::IpAddr>()
        .map(|address| address.is_loopback())
        .unwrap_or(false)
}

/// Whether `name` is usable as an environment variable name.
fn is_valid_env_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Whether `id` looks like a Discord snowflake ID: digits only, no leading zero.
pub fn is_snowflake_like(id: &str) -> bool {
    let id = id.trim();
    id.len() <= MAX_SNOWFLAKE_LEN
        && id
            .bytes()
            .next()
            .is_some_and(|first| first.is_ascii_digit() && first != b'0')
        && id.bytes().all(|byte| byte.is_ascii_digit())
}

fn validate_allowlist(field: &'static str, ids: &[String]) -> Result<(), ConfigError> {
    for id in ids {
        if !is_snowflake_like(id) {
            return Err(ConfigError::invalid(
                field,
                format!(
                    "contains `{}`, which is not a Discord snowflake ID (digits only)",
                    id.trim()
                ),
            ));
        }
    }
    Ok(())
}
