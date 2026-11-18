//! Application-facing Discord data.
//!
//! These are the only Discord shapes the rest of the crate sees. twilight's
//! models stay inside [`crate::discord::adapter`], so a crate upgrade cannot
//! ripple into the MCP layer, and the fields exposed here are deliberately a
//! subset: only what a tool result needs.

use serde::Serialize;

use crate::discord::ids::{ChannelId, GuildId, MessageId, UserId};

/// The bot account this server authenticates as.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BotIdentity {
    /// Bot user ID.
    pub id: UserId,
    /// Bot username.
    pub username: String,
}

/// One guild the bot is a member of.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GuildSummary {
    /// Guild ID.
    pub id: GuildId,
    /// Guild name.
    pub name: String,
    /// Icon hash, if the guild has an icon.
    pub icon_hash: Option<String>,
    /// Whether the bot owns this guild.
    pub owner: bool,
}

/// Guild metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GuildDetail {
    /// Guild ID.
    pub id: GuildId,
    /// Guild name.
    pub name: String,
    /// Icon hash, if any.
    pub icon_hash: Option<String>,
    /// Guild description, if set.
    pub description: Option<String>,
    /// Owner's user ID.
    pub owner_id: Option<UserId>,
    /// Member count, when Discord reports it.
    pub member_count: Option<u64>,
}

/// The channel types this server distinguishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelKind {
    /// Standard text channel.
    Text,
    /// Voice channel.
    Voice,
    /// Channel category.
    Category,
    /// Announcement channel.
    Announcement,
    /// Forum channel.
    Forum,
    /// Stage voice channel.
    Stage,
    /// Thread of any kind.
    Thread,
    /// Directory channel.
    Directory,
    /// A type this build does not name.
    Other(u8),
}

impl Serialize for ChannelKind {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl ChannelKind {
    /// Map Discord's numeric channel type.
    pub const fn from_code(code: u8) -> Self {
        match code {
            0 => Self::Text,
            2 => Self::Voice,
            4 => Self::Category,
            5 => Self::Announcement,
            10..=12 => Self::Thread,
            13 => Self::Stage,
            14 => Self::Directory,
            15 => Self::Forum,
            other => Self::Other(other),
        }
    }

    /// The configuration spelling of this kind.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Voice => "voice",
            Self::Category => "category",
            Self::Announcement => "announcement",
            Self::Forum => "forum",
            Self::Stage => "stage",
            Self::Thread => "thread",
            Self::Directory => "directory",
            Self::Other(_) => "other",
        }
    }
}

/// Channel metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChannelSummary {
    /// Channel ID.
    pub id: ChannelId,
    /// Guild this channel belongs to, when it belongs to one.
    pub guild_id: Option<GuildId>,
    /// Channel name. Threads and DMs may not have one.
    pub name: Option<String>,
    /// Channel type.
    pub kind: ChannelKind,
    /// Sort position within the guild.
    pub position: Option<i32>,
    /// Parent category or channel, when nested.
    pub parent_id: Option<ChannelId>,
    /// Channel topic, when set.
    pub topic: Option<String>,
    /// Whether the channel is age restricted.
    pub nsfw: bool,
}

/// Who wrote a message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MessageAuthor {
    /// Author's user ID.
    pub id: UserId,
    /// Author's username.
    pub username: String,
    /// Whether the author is a bot account.
    pub bot: bool,
}

/// One message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MessageSummary {
    /// Message ID.
    pub id: MessageId,
    /// Channel the message is in.
    pub channel_id: ChannelId,
    /// Guild the message is in, for guild channels.
    pub guild_id: Option<GuildId>,
    /// Message author.
    pub author: MessageAuthor,
    /// Message body.
    ///
    /// `None` when Discord returned no content: empty messages, and message
    /// bodies withheld because the bot lacks the Message Content intent.
    pub content: Option<String>,
    /// Creation timestamp, ISO 8601.
    pub timestamp: String,
    /// Last edit timestamp, ISO 8601, when edited.
    pub edited_timestamp: Option<String>,
    /// Number of attachments. Attachment details are not exposed.
    pub attachment_count: usize,
}

/// One guild member.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MemberSummary {
    /// Member's user ID.
    pub user_id: UserId,
    /// Username.
    pub username: String,
    /// Guild-specific nickname, when set.
    pub nickname: Option<String>,
    /// Role IDs held by the member.
    pub roles: Vec<String>,
    /// Join timestamp, ISO 8601, when reported.
    pub joined_at: Option<String>,
}

/// One bounded page of results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Page<T> {
    /// The items in this page, in the order Discord returned them.
    pub items: Vec<T>,
    /// Opaque cursor for the next page, when one is available.
    ///
    /// Callers pass this back as the tool's paging cursor — `after` for
    /// ascending listings, `before` for message history — and must not
    /// interpret it.
    pub next_cursor: Option<String>,
}

impl<T> Page<T> {
    /// Build a page.
    pub fn new(items: Vec<T>, next_cursor: Option<String>) -> Self {
        Self { items, next_cursor }
    }

    /// Whether this page has no items.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// A bounded request for channel history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageQuery {
    /// Maximum messages to return. Clamped to Discord's 100-message page limit.
    pub limit: u16,
    /// Return messages older than this ID.
    pub before: Option<MessageId>,
    /// Return messages newer than this ID.
    pub after: Option<MessageId>,
}

impl MessageQuery {
    /// A newest-first page of `limit` messages.
    pub const fn latest(limit: u16) -> Self {
        Self {
            limit,
            before: None,
            after: None,
        }
    }
}
