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
    /// Changes server-wide configuration in bulk.
    Administrative,
}

impl ToolRisk {
    /// Stable spelling for diagnostics.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Destructive => "destructive",
            Self::Administrative => "administrative",
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
        | "discord_get_member"
        | "discord_get_capabilities" => Some(ToolRisk::Read),
        "discord_send_message"
        | "discord_edit_message"
        | "discord_add_reaction"
        | "discord_remove_reaction"
        | "discord_clear_reactions"
        | "discord_pin_message"
        | "discord_unpin_message" => Some(ToolRisk::Write),
        "discord_delete_message"
        | "discord_create_channel"
        | "discord_edit_channel"
        | "discord_delete_channel"
        | "discord_set_permission_overwrite"
        | "discord_delete_permission_overwrite"
        | "discord_create_thread"
        | "discord_edit_thread"
        | "discord_create_invite"
        | "discord_delete_invite" => Some(ToolRisk::Destructive),
        "discord_bulk_delete_messages" => Some(ToolRisk::Administrative),
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
    control_enabled: bool,
    administrative_enabled: bool,
    action_budget: u16,
    max_bulk_delete: u16,
    require_dry_run: Vec<String>,
}

impl Policy {
    /// Compile validated configuration into a policy.
    ///
    /// Allowlist parsing is repeated here rather than assumed: a policy that
    /// cannot parse its own allowlist is a configuration failure, and silently
    /// treating it as empty would widen authority instead of narrowing it.
    pub fn from_config(config: &PolicyConfig) -> Result<Self, AppError> {
        // A name that is not a tool would make `require_dry_run` silently inert,
        // so it is a configuration error rather than a no-op.
        let require_dry_run = config
            .autonomy
            .require_dry_run
            .iter()
            .map(|tool| {
                let name = tool.trim();
                if classify_tool(name).is_none() {
                    return Err(AppError::configuration(format!(
                        "policy.autonomy.require_dry_run names `{name}`, which is not a tool this build classifies"
                    )));
                }
                Ok(name.to_string())
            })
            .collect::<Result<Vec<_>, AppError>>()?;

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
            control_enabled: config.control_enabled,
            administrative_enabled: config.administrative_enabled,
            action_budget: config.autonomy.max_actions_per_minute,
            max_bulk_delete: config.autonomy.max_bulk_delete,
            require_dry_run,
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

    /// Refuse every control tool unless `policy.control_enabled`.
    pub fn check_control_enabled(&self) -> Result<(), AppError> {
        if self.control_enabled {
            Ok(())
        } else {
            Err(AppError::policy_blocked(
                "control tools are disabled by policy.control_enabled",
            ))
        }
    }

    /// Refuse every administrative tool unless `policy.administrative_enabled`.
    pub fn check_administrative_enabled(&self) -> Result<(), AppError> {
        if self.administrative_enabled {
            Ok(())
        } else {
            Err(AppError::policy_blocked(
                "administrative tools are disabled by policy.administrative_enabled",
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
            ToolRisk::Destructive => self.check_control_enabled()?,
            ToolRisk::Administrative => self.check_administrative_enabled()?,
        }
        Ok(risk)
    }

    /// Mutating actions permitted in any rolling minute.
    pub const fn action_budget(&self) -> u16 {
        self.action_budget
    }

    /// Largest accepted bulk-delete batch.
    pub const fn max_bulk_delete(&self) -> u16 {
        self.max_bulk_delete
    }

    /// Whether `action` must be called with `dry_run = true`.
    pub fn requires_dry_run(&self, action: &str) -> bool {
        self.require_dry_run.iter().any(|tool| tool == action)
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

    /// Whether a channel allowlist narrows the scope at all.
    pub fn channel_restricted(&self) -> bool {
        !self.channels.is_empty()
    }

    /// Whether a channel may be touched, without building an error.
    pub fn allows_channel(&self, channel_id: &ChannelId) -> bool {
        self.check_channel(channel_id).is_ok()
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

    /// Validate and sanitise a message this server would send or edit.
    ///
    /// Shared by `discord_send_message` and `discord_edit_message`: an edit is
    /// subject to the same length limit and mention neutralisation as a send, so
    /// it cannot be used to smuggle a notification.
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
