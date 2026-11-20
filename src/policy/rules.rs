//! Policy rules: the decisions themselves.
//!
//! Everything here is a function of local configuration plus the request. There
//! is no I/O, no async and no Discord client, which is what makes the authority
//! boundary testable in isolation and impossible for Discord message content to
//! influence: nothing in this module reads a message.

use std::path::{Component, Path, PathBuf};

use crate::config::schema::PolicyConfig;
use crate::discord::ids::{ChannelId, GuildId, RoleId};
use crate::discord::models::NewEmbed;
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
    /// Restricts or removes a guild member.
    Moderation,
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
            Self::Moderation => "moderation",
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
        | "discord_get_user_messages"
        | "discord_find_abusive_messages"
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
        "discord_set_member_nickname"
        | "discord_kick_member"
        | "discord_ban_member"
        | "discord_unban_member"
        | "discord_timeout_member"
        | "discord_clear_member_timeout" => Some(ToolRisk::Moderation),
        "discord_bulk_delete_messages"
        | "discord_create_role"
        | "discord_edit_role"
        | "discord_delete_role"
        | "discord_assign_role"
        | "discord_remove_role" => Some(ToolRisk::Administrative),
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
    /// Sorted and de-duplicated; empty means "no guild restriction".
    guilds: Vec<GuildId>,
    /// Sorted and de-duplicated; empty means "no channel restriction".
    channels: Vec<ChannelId>,
    /// Channels no tool may edit or delete, independent of the allowlist.
    protected_channels: Vec<ChannelId>,
    /// Roles no tool may edit or delete, independent of the guild allowlist.
    protected_roles: Vec<RoleId>,
    max_messages_per_read: u16,
    max_message_characters: u16,
    max_embeds_per_message: u16,
    max_embed_characters: u16,
    max_attachments_per_message: u16,
    max_attachment_bytes: u64,
    max_attachment_total_bytes: u64,
    attachment_root: Option<PathBuf>,
    allow_mass_mentions: bool,
    max_concurrent_requests: u16,
    control_enabled: bool,
    moderation_enabled: bool,
    administrative_enabled: bool,
    action_budget: u16,
    send_budget: u16,
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
            protected_channels: parse_ids(
                "policy.protected_channels",
                &config.protected_channels,
                ChannelId::parse,
            )?,
            protected_roles: parse_ids(
                "policy.protected_roles",
                &config.protected_roles,
                RoleId::parse,
            )?,
            max_messages_per_read: config.max_messages_per_read,
            max_message_characters: config.max_message_characters,
            max_embeds_per_message: config.max_embeds_per_message,
            max_embed_characters: config.max_embed_characters,
            max_attachments_per_message: config.max_attachments_per_message,
            max_attachment_bytes: config.max_attachment_bytes,
            max_attachment_total_bytes: config.max_attachment_total_bytes,
            attachment_root: config.attachment_root.as_deref().map(PathBuf::from),
            allow_mass_mentions: config.allow_mass_mentions,
            max_concurrent_requests: config.max_concurrent_requests,
            control_enabled: config.control_enabled,
            moderation_enabled: config.moderation_enabled,
            administrative_enabled: config.administrative_enabled,
            action_budget: config.autonomy.max_actions_per_minute,
            send_budget: config.autonomy.max_sends_per_channel_per_minute,
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

    /// Refuse every member-moderation tool unless `policy.moderation_enabled`.
    pub fn check_moderation_enabled(&self) -> Result<(), AppError> {
        if self.moderation_enabled {
            Ok(())
        } else {
            Err(AppError::policy_blocked(
                "moderation tools are disabled by policy.moderation_enabled",
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
            ToolRisk::Moderation => self.check_moderation_enabled()?,
            ToolRisk::Administrative => self.check_administrative_enabled()?,
        }
        Ok(risk)
    }

    /// Mutating actions permitted in any rolling minute.
    pub const fn action_budget(&self) -> u16 {
        self.action_budget
    }

    /// Messages one channel may receive per rolling minute.
    pub const fn send_budget(&self) -> u16 {
        self.send_budget
    }

    /// Whether a channel is protected from edit and deletion.
    pub fn check_protected_channel(&self, channel_id: &ChannelId) -> Result<(), AppError> {
        if self.protected_channels.binary_search(channel_id).is_ok() {
            return Err(AppError::policy_blocked(format!(
                "channel {channel_id} is protected by policy.protected_channels and cannot be edited or deleted"
            )));
        }
        Ok(())
    }

    /// Whether a role is protected from edit and deletion.
    pub fn check_protected_role(&self, role_id: &RoleId) -> Result<(), AppError> {
        if self.protected_roles.binary_search(role_id).is_ok() {
            return Err(AppError::policy_blocked(format!(
                "role {role_id} is protected by policy.protected_roles and cannot be edited or deleted"
            )));
        }
        Ok(())
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
        self.prepare_content(content)
    }

    /// Validate optional send text. `None` is allowed when an embed or file
    /// carries the message; whitespace alone is treated as no text.
    pub fn prepare_send_content(
        &self,
        content: Option<&str>,
    ) -> Result<Option<SanitizedMessage>, AppError> {
        self.check_write_enabled()?;
        match content {
            Some(content) if !content.trim().is_empty() => self.prepare_content(content).map(Some),
            _ => Ok(None),
        }
    }

    fn prepare_content(&self, content: &str) -> Result<SanitizedMessage, AppError> {
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

    /// Validate a send's embeds against the configured bounds and Discord's
    /// per-field limits.
    pub fn check_embeds(&self, embeds: &[NewEmbed]) -> Result<(), AppError> {
        if embeds.len() > usize::from(self.max_embeds_per_message) {
            return Err(AppError::policy_blocked(format!(
                "message has {} embeds; policy.max_embeds_per_message is {}",
                embeds.len(),
                self.max_embeds_per_message
            )));
        }
        if self.max_embeds_per_message == 0 && !embeds.is_empty() {
            return Err(AppError::policy_blocked(
                "embeds are disabled by policy.max_embeds_per_message = 0",
            ));
        }

        let mut total = 0usize;
        for (index, embed) in embeds.iter().enumerate() {
            validate_embed(index, embed)?;
            total += embed_character_count(embed)?;
        }
        if total > usize::from(self.max_embed_characters) {
            return Err(AppError::policy_blocked(format!(
                "message embeds contain {total} characters; policy.max_embed_characters is {}",
                self.max_embed_characters
            )));
        }
        Ok(())
    }

    /// Reject a send that carries no text, embeds or files.
    pub fn check_send_payload(
        &self,
        content: Option<&SanitizedMessage>,
        embeds: &[NewEmbed],
        attachment_count: usize,
    ) -> Result<(), AppError> {
        if content.is_none() && embeds.is_empty() && attachment_count == 0 {
            return Err(AppError::invalid_argument(
                "message must carry content, at least one embed, or at least one file",
            ));
        }
        Ok(())
    }

    /// Check an attachment count before resolving paths.
    fn check_attachment_count(&self, count: usize) -> Result<(), AppError> {
        if count > usize::from(self.max_attachments_per_message) {
            return Err(AppError::policy_blocked(format!(
                "message has {count} files; policy.max_attachments_per_message is {}",
                self.max_attachments_per_message
            )));
        }
        if self.max_attachments_per_message == 0 && count > 0 {
            return Err(AppError::policy_blocked(
                "file uploads are disabled by policy.max_attachments_per_message = 0",
            ));
        }
        Ok(())
    }

    /// Resolve one untrusted relative file path beneath the configured root.
    ///
    /// This deliberately rejects absolute paths, parent-directory components and
    /// symlinks that canonicalise outside the root. The metadata checks are
    /// repeated against the canonical target, not the lexical path.
    pub fn resolve_attachment_path(&self, index: usize, path: &str) -> Result<PathBuf, AppError> {
        let Some(root) = &self.attachment_root else {
            return Err(AppError::policy_blocked(
                "file uploads are disabled; set policy.attachment_root to enable them",
            ));
        };
        let relative = Path::new(path);
        if relative.as_os_str().is_empty() {
            return Err(AppError::invalid_argument(format!(
                "file {index} has an empty path"
            )));
        }
        if relative.is_absolute()
            || relative
                .components()
                .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
        {
            return Err(AppError::invalid_argument(format!(
                "file {index} must be a relative path without `..` or a drive prefix"
            )));
        }

        let root = std::fs::canonicalize(root).map_err(|error| {
            AppError::configuration(format!(
                "policy.attachment_root cannot be resolved: {error}"
            ))
        })?;
        let candidate = root.join(relative);
        let canonical = std::fs::canonicalize(&candidate).map_err(|_| {
            AppError::invalid_argument(format!("file {index} does not exist or is not readable"))
        })?;
        if !canonical.starts_with(&root) {
            return Err(AppError::policy_blocked(format!(
                "file {index} resolves outside policy.attachment_root"
            )));
        }
        let metadata = std::fs::metadata(&canonical)
            .map_err(|_| AppError::invalid_argument(format!("file {index} is not readable")))?;
        if !metadata.is_file() {
            return Err(AppError::invalid_argument(format!(
                "file {index} is not a regular file"
            )));
        }
        if metadata.len() > self.max_attachment_bytes {
            return Err(AppError::policy_blocked(format!(
                "file {index} is {} bytes; policy.max_attachment_bytes is {}",
                metadata.len(),
                self.max_attachment_bytes
            )));
        }
        Ok(canonical)
    }

    /// Resolve and bound every file on a send.
    pub fn resolve_attachments(&self, paths: &[PathBuf]) -> Result<Vec<PathBuf>, AppError> {
        self.check_attachment_count(paths.len())?;
        let mut resolved = Vec::with_capacity(paths.len());
        let mut total = 0u64;
        for (index, path) in paths.iter().enumerate() {
            let canonical = self.resolve_attachment_path(index, &path.to_string_lossy())?;
            let length = std::fs::metadata(&canonical)
                .map_err(|_| AppError::invalid_argument(format!("file {index} is not readable")))?
                .len();
            total = total
                .checked_add(length)
                .ok_or_else(|| AppError::policy_blocked("total upload size overflowed"))?;
            if total > self.max_attachment_total_bytes {
                return Err(AppError::policy_blocked(format!(
                    "files total {total} bytes; policy.max_attachment_total_bytes is {}",
                    self.max_attachment_total_bytes
                )));
            }
            resolved.push(canonical);
        }
        Ok(resolved)
    }

    /// The concurrency ceiling for in-flight Discord requests.
    pub const fn max_concurrent_requests(&self) -> u16 {
        self.max_concurrent_requests
    }
}

const EMBED_TITLE_MAX: usize = 256;
const EMBED_DESCRIPTION_MAX: usize = 4_096;
const EMBED_FIELD_NAME_MAX: usize = 256;
const EMBED_FIELD_VALUE_MAX: usize = 1_024;
const EMBED_FOOTER_MAX: usize = 2_048;
const EMBED_URL_MAX: usize = 2_048;
const EMBED_FIELDS_MAX: usize = 25;

fn validate_embed(index: usize, embed: &NewEmbed) -> Result<(), AppError> {
    if embed.title.is_none()
        && embed.description.is_none()
        && embed.url.is_none()
        && embed.fields.is_empty()
        && embed.footer.is_none()
        && embed.image_url.is_none()
        && embed.thumbnail_url.is_none()
    {
        return Err(AppError::invalid_argument(format!(
            "embed {index} is empty; give it a title, description, field, footer or image"
        )));
    }
    if embed.fields.len() > EMBED_FIELDS_MAX {
        return Err(AppError::invalid_argument(format!(
            "embed {index} has {} fields; Discord allows at most {EMBED_FIELDS_MAX}",
            embed.fields.len()
        )));
    }
    if let Some(color) = embed.color
        && color > 0x00ff_ffff
    {
        return Err(AppError::invalid_argument(format!(
            "embed {index} colour must be between 0x000000 and 0xFFFFFF"
        )));
    }

    check_optional_text(index, "title", embed.title.as_deref(), EMBED_TITLE_MAX)?;
    check_optional_text(
        index,
        "description",
        embed.description.as_deref(),
        EMBED_DESCRIPTION_MAX,
    )?;
    check_optional_text(index, "footer", embed.footer.as_deref(), EMBED_FOOTER_MAX)?;
    check_optional_url(index, "url", embed.url.as_deref(), false)?;
    check_optional_url(index, "image_url", embed.image_url.as_deref(), true)?;
    check_optional_url(index, "thumbnail_url", embed.thumbnail_url.as_deref(), true)?;

    for (field_index, field) in embed.fields.iter().enumerate() {
        let name = field.name.trim();
        let value = field.value.trim();
        if name.is_empty() || value.is_empty() {
            return Err(AppError::invalid_argument(format!(
                "embed {index} field {field_index} must have a non-empty name and value"
            )));
        }
        let name_length = name.chars().count();
        if name_length > EMBED_FIELD_NAME_MAX {
            return Err(AppError::invalid_argument(format!(
                "embed {index} field {field_index} name is {name_length} characters; the maximum is {EMBED_FIELD_NAME_MAX}"
            )));
        }
        let value_length = value.chars().count();
        if value_length > EMBED_FIELD_VALUE_MAX {
            return Err(AppError::invalid_argument(format!(
                "embed {index} field {field_index} value is {value_length} characters; the maximum is {EMBED_FIELD_VALUE_MAX}"
            )));
        }
    }
    Ok(())
}

fn check_optional_text(
    index: usize,
    label: &str,
    value: Option<&str>,
    max: usize,
) -> Result<(), AppError> {
    let Some(value) = value else {
        return Ok(());
    };
    if value.trim().is_empty() {
        return Err(AppError::invalid_argument(format!(
            "embed {index} {label} must not be empty when set"
        )));
    }
    let length = value.chars().count();
    if length > max {
        return Err(AppError::invalid_argument(format!(
            "embed {index} {label} is {length} characters; the maximum is {max}"
        )));
    }
    Ok(())
}

fn check_optional_url(
    index: usize,
    label: &str,
    value: Option<&str>,
    allow_attachment: bool,
) -> Result<(), AppError> {
    let Some(value) = value else {
        return Ok(());
    };
    if value.trim().is_empty() {
        return Err(AppError::invalid_argument(format!(
            "embed {index} {label} must not be empty when set"
        )));
    }
    let length = value.chars().count();
    if length > EMBED_URL_MAX {
        return Err(AppError::invalid_argument(format!(
            "embed {index} {label} is {length} characters; the maximum is {EMBED_URL_MAX}"
        )));
    }
    let scheme_ok = value.starts_with("http://")
        || value.starts_with("https://")
        || (allow_attachment && value.starts_with("attachment://"));
    if !scheme_ok {
        return Err(AppError::invalid_argument(format!(
            "embed {index} {label} must be an http(s) URL{}",
            if allow_attachment {
                " or an attachment:// reference"
            } else {
                ""
            }
        )));
    }
    Ok(())
}

fn embed_character_count(embed: &NewEmbed) -> Result<usize, AppError> {
    let mut total = 0usize;
    for value in [
        embed.title.as_deref(),
        embed.description.as_deref(),
        embed.url.as_deref(),
        embed.footer.as_deref(),
        embed.image_url.as_deref(),
        embed.thumbnail_url.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        total = total
            .checked_add(value.chars().count())
            .ok_or_else(|| AppError::invalid_argument("embed text is too large to count"))?;
    }
    for field in &embed.fields {
        total = total
            .checked_add(field.name.chars().count())
            .and_then(|value| value.checked_add(field.value.chars().count()))
            .ok_or_else(|| AppError::invalid_argument("embed text is too large to count"))?;
    }
    Ok(total)
}

const MEMBER_NICKNAME_MAX: usize = 32;
const MODERATION_REASON_MAX: usize = 512;
const BAN_DELETE_SECONDS_MAX: u32 = 604_800;
const TIMEOUT_SECONDS_MAX: u32 = 2_419_200;

/// Validate a moderation nickname. An exactly empty string clears the nickname;
/// whitespace alone is refused so a missing value cannot silently clear it.
pub fn validate_member_nickname(nickname: &str) -> Result<Option<String>, AppError> {
    if nickname.is_empty() {
        return Ok(None);
    }
    let nickname = nickname.trim();
    if nickname.is_empty() {
        return Err(AppError::invalid_argument(
            "`nickname` must not be whitespace only; use an empty string to clear it",
        ));
    }
    let length = nickname.chars().count();
    if length > MEMBER_NICKNAME_MAX {
        return Err(AppError::invalid_argument(format!(
            "`nickname` is {length} characters; Discord allows at most {MEMBER_NICKNAME_MAX}"
        )));
    }
    if nickname.chars().any(char::is_control) {
        return Err(AppError::invalid_argument(
            "`nickname` must not contain control characters",
        ));
    }
    Ok(Some(nickname.to_string()))
}

/// Validate an optional Discord audit-log reason. The cap and the rejection of
/// header-breaking characters are local, so no request is built for a bad one.
pub fn validate_moderation_reason(reason: Option<&str>) -> Result<Option<String>, AppError> {
    let Some(reason) = reason else {
        return Ok(None);
    };
    let reason = reason.trim();
    if reason.is_empty() {
        return Ok(None);
    }
    let length = reason.chars().count();
    if length > MODERATION_REASON_MAX {
        return Err(AppError::invalid_argument(format!(
            "`reason` is {length} characters; Discord allows at most {MODERATION_REASON_MAX}"
        )));
    }
    if reason
        .chars()
        .any(|character| matches!(character, '\r' | '\n' | '\0'))
    {
        return Err(AppError::invalid_argument(
            "`reason` must not contain newlines or NUL characters",
        ));
    }
    Ok(Some(reason.to_string()))
}

/// Validate Discord's ban message-deletion window: zero means keep messages.
pub fn validate_ban_delete_seconds(seconds: u32) -> Result<(), AppError> {
    if seconds > BAN_DELETE_SECONDS_MAX {
        return Err(AppError::invalid_argument(format!(
            "`delete_message_seconds` must be at most {BAN_DELETE_SECONDS_MAX} (7 days), got {seconds}"
        )));
    }
    Ok(())
}

/// Validate Discord's timeout limit: one second through 28 days.
pub fn validate_timeout_seconds(seconds: u32) -> Result<(), AppError> {
    if !(1..=TIMEOUT_SECONDS_MAX).contains(&seconds) {
        return Err(AppError::invalid_argument(format!(
            "`duration_seconds` must be between 1 and {TIMEOUT_SECONDS_MAX} (28 days), got {seconds}"
        )));
    }
    Ok(())
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
