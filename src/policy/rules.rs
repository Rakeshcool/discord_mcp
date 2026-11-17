//! Policy rules: the decisions themselves.
//!
//! Everything here is a function of local configuration plus the request. There
//! is no I/O, no async and no Discord client, which is what makes the authority
//! boundary testable in isolation and impossible for Discord message content to
//! influence: nothing in this module reads a message.

use crate::config::schema::PolicyConfig;
use crate::discord::ids::{ChannelId, GuildId};
use crate::error::AppError;

/// Inserted into a mention so Discord neither resolves nor notifies it.
const ZERO_WIDTH_SPACE: char = '\u{200b}';

/// How dangerous a tool is, which decides what must hold before it may run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ToolRisk {
    /// Reads Discord data.
    Read,
    /// Sends data to Discord.
    Write,
    /// Deletes or irreversibly changes data.
    Destructive,
}

impl ToolRisk {
    /// Stable spelling for diagnostics.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Destructive => "destructive",
        }
    }
}

/// Classify a tool by its MCP name.
///
/// `None` means "not a tool this build knows". Callers must treat that as a
/// denial, never as a default permission.
pub fn classify_tool(name: &str) -> Option<ToolRisk> {
    match name {
        "discord_list_guilds"
        | "discord_get_guild"
        | "discord_list_channels"
        | "discord_get_channel"
        | "discord_get_messages"
        | "discord_get_message"
        | "discord_list_members"
        | "discord_get_member" => Some(ToolRisk::Read),
        "discord_send_message" => Some(ToolRisk::Write),
        _ => None,
    }
}

/// Message content after mention neutralisation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SanitizedMessage {
    /// Content that is safe to send.
    pub content: String,
    /// How many mentions were neutralised.
    pub suppressed_mentions: usize,
}

impl SanitizedMessage {
    /// Whether neutralisation changed anything.
    pub const fn changed(&self) -> bool {
        self.suppressed_mentions > 0
    }
}

/// Break every mention in `content` that could notify someone.
///
/// A mention is neutralised by inserting a zero-width space, which stops Discord
/// resolving it while leaving the message readable. With `allow_mentions` the
/// content passes through untouched; configuration currently refuses to set that
/// flag, so the neutralising path is the one that ships.
pub fn sanitize_mentions(content: &str, allow_mentions: bool) -> SanitizedMessage {
    if allow_mentions {
        return SanitizedMessage {
            content: content.to_string(),
            suppressed_mentions: 0,
        };
    }

    let mut out = String::with_capacity(content.len() + 8);
    let mut suppressed = 0usize;
    let mut index = 0usize;

    while index < content.len() {
        let rest = &content[index..];
        // `@everyone`/`@here` notify by plain text.
        if rest.starts_with("@everyone") {
            out.push('@');
            out.push(ZERO_WIDTH_SPACE);
            out.push_str("everyone");
            index += "@everyone".len();
            suppressed += 1;
            continue;
        }
        if rest.starts_with("@here") {
            out.push('@');
            out.push(ZERO_WIDTH_SPACE);
            out.push_str("here");
            index += "@here".len();
            suppressed += 1;
            continue;
        }
        // `<@id>`, `<@!id>` and `<@&id>` cover user and role mentions.
        if rest.starts_with("<@") {
            out.push('<');
            out.push(ZERO_WIDTH_SPACE);
            out.push('@');
            index += 2;
            suppressed += 1;
            continue;
        }

        let Some(character) = rest.chars().next() else {
            break;
        };
        out.push(character);
        index += character.len_utf8();
    }

    SanitizedMessage {
        content: out,
        suppressed_mentions: suppressed,
    }
}

/// The effective authority boundary: validated configuration compiled into
/// decisions that need no further parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    read_enabled: bool,
    write_enabled: bool,
    require_confirmation_for_destructive: bool,
    /// Sorted and de-duplicated; empty means "no guild restriction".
    guilds: Vec<GuildId>,
    /// Sorted and de-duplicated; empty means "no channel restriction".
    channels: Vec<ChannelId>,
    max_messages_per_read: u16,
    max_message_characters: u16,
    allow_mass_mentions: bool,
    max_concurrent_requests: u16,
}

impl Policy {
    /// Compile validated configuration into a policy.
    ///
    /// Allowlist parsing is repeated here rather than assumed: a policy that
    /// cannot parse its own allowlist is a configuration failure, and silently
    /// treating it as empty would widen authority instead of narrowing it.
    pub fn from_config(config: &PolicyConfig) -> Result<Self, AppError> {
        Ok(Self {
            read_enabled: config.read_enabled,
            write_enabled: config.write_enabled,
            require_confirmation_for_destructive: config.require_confirmation_for_destructive,
            guilds: parse_ids(
                "policy.guild_allowlist",
                &config.guild_allowlist,
                GuildId::parse,
            )?,
            channels: parse_ids(
                "policy.channel_allowlist",
                &config.channel_allowlist,
                ChannelId::parse,
            )?,
            max_messages_per_read: config.max_messages_per_read,
            max_message_characters: config.max_message_characters,
            allow_mass_mentions: config.allow_mass_mentions,
            max_concurrent_requests: config.max_concurrent_requests,
        })
    }

    /// Refuse every read tool unless `policy.read_enabled`.
    pub fn check_read_enabled(&self) -> Result<(), AppError> {
        if self.read_enabled {
            Ok(())
        } else {
            Err(AppError::policy_blocked(
                "read tools are disabled by policy.read_enabled",
            ))
        }
    }

    /// Refuse every write tool unless `policy.write_enabled`.
    pub fn check_write_enabled(&self) -> Result<(), AppError> {
        if self.write_enabled {
            Ok(())
        } else {
            Err(AppError::policy_blocked(
                "write tools are disabled by policy.write_enabled",
            ))
        }
    }

    /// Classify a tool and check that its risk class is enabled.
    ///
    /// An unclassified tool is a denial: the registry must never fall back to
    /// "unknown means allowed".
    pub fn check_tool(&self, name: &str) -> Result<ToolRisk, AppError> {
        let risk = classify_tool(name).ok_or_else(|| {
            AppError::policy_blocked(format!("`{name}` is not a tool this build classifies"))
        })?;
        match risk {
            ToolRisk::Read => self.check_read_enabled()?,
            ToolRisk::Write => self.check_write_enabled()?,
            ToolRisk::Destructive => {
                return Err(AppError::policy_blocked(format!(
                    "`{name}` is destructive and no approval channel exists"
                )));
            }
        }
        Ok(risk)
    }

    /// Whether a guild allowlist narrows the scope at all.
    pub fn guild_restricted(&self) -> bool {
        !self.guilds.is_empty()
    }

    /// Whether a guild may be touched.
    pub fn check_guild(&self, guild_id: &GuildId) -> Result<(), AppError> {
        if self.guilds.is_empty() || self.guilds.binary_search(guild_id).is_ok() {
            return Ok(());
        }
        Err(AppError::policy_blocked(format!(
            "guild {guild_id} is not on policy.guild_allowlist"
        )))
    }

    /// Whether a guild may be touched, without building an error.
    pub fn allows_guild(&self, guild_id: &GuildId) -> bool {
        self.check_guild(guild_id).is_ok()
    }

    /// Whether a channel may be touched.
    pub fn check_channel(&self, channel_id: &ChannelId) -> Result<(), AppError> {
        if self.channels.is_empty() || self.channels.binary_search(channel_id).is_ok() {
            return Ok(());
        }
        Err(AppError::policy_blocked(format!(
            "channel {channel_id} is not on policy.channel_allowlist"
        )))
    }

    /// Reject a page size the policy does not permit.
    ///
    /// The adapter clamps to Discord's physical maximum; this is an authority
    /// limit, so exceeding it is an error rather than a silent truncation the
    /// caller would never learn about.
    pub fn check_page_limit(&self, requested: u16) -> Result<(), AppError> {
        if requested == 0 {
            return Err(AppError::invalid_argument("`limit` must be at least 1"));
        }
        if requested > self.max_messages_per_read {
            return Err(AppError::invalid_argument(format!(
                "`limit` {requested} exceeds policy.max_messages_per_read ({})",
                self.max_messages_per_read
            )));
        }
        Ok(())
    }

    /// Validate and sanitise a message this server would send.
    ///
    /// Unused until the write tool lands, which is exactly why it is written and
    /// tested now: the write tool must not be the first place message safety is
    /// considered.
    pub fn prepare_outgoing_message(&self, content: &str) -> Result<SanitizedMessage, AppError> {
        self.check_write_enabled()?;
        if content.trim().is_empty() {
            return Err(AppError::invalid_argument(
                "message content must not be empty",
            ));
        }
        let length = content.chars().count();
        if length > usize::from(self.max_message_characters) {
            return Err(AppError::invalid_argument(format!(
                "message content is {length} characters; policy.max_message_characters is {}",
                self.max_message_characters
            )));
        }
        Ok(sanitize_mentions(content, self.allow_mass_mentions))
    }

    /// Whether destructive actions require confirmation.
    pub const fn require_confirmation_for_destructive(&self) -> bool {
        self.require_confirmation_for_destructive
    }

    /// The concurrency ceiling for in-flight Discord requests.
    pub const fn max_concurrent_requests(&self) -> u16 {
        self.max_concurrent_requests
    }
}

fn parse_ids<T: Ord>(
    field: &str,
    values: &[String],
    parse: impl Fn(&str) -> Result<T, AppError>,
) -> Result<Vec<T>, AppError> {
    let mut parsed = Vec::with_capacity(values.len());
    for value in values {
        let id = parse(value).map_err(|_| {
            AppError::configuration(format!(
                "{field} contains `{}`, which is not a Discord snowflake ID",
                value.trim()
            ))
        })?;
        parsed.push(id);
    }
    parsed.sort_unstable();
    parsed.dedup();
    Ok(parsed)
}
