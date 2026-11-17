//! The application-owned Discord interface.
//!
//! The application layer depends on this trait, never on twilight. That keeps
//! the adapter replaceable, makes the whole read surface testable without a
//! network, and is what lets the policy layer (next phase) sit between tool
//! arguments and real requests.
//!
//! Every method is a read. Writes arrive in a later phase behind separate
//! policy checks, and no method here can create, modify or delete anything.

use async_trait::async_trait;

use crate::discord::ids::{ChannelId, GuildId, MessageId, UserId};
use crate::discord::models::{
    ChannelSummary, GuildDetail, GuildSummary, MemberSummary, MessageQuery, MessageSummary, Page,
};
use crate::error::AppError;

/// Discord's maximum page size for `GET /users/@me/guilds`.
pub const GUILDS_PAGE_MAX: u16 = 200;

/// Discord's maximum page size for channel messages.
pub const MESSAGES_PAGE_MAX: u16 = 100;

/// Discord's maximum page size for guild members.
pub const MEMBERS_PAGE_MAX: u16 = 1000;

/// Read-only Discord operations, expressed in this crate's own types.
#[async_trait]
pub trait DiscordApi: Send + Sync + std::fmt::Debug {
    /// Guilds the bot is a member of, ascending by ID.
    async fn list_guilds(
        &self,
        limit: u16,
        after: Option<&GuildId>,
    ) -> Result<Page<GuildSummary>, AppError>;

    /// Metadata for one guild.
    async fn get_guild(&self, guild_id: &GuildId) -> Result<GuildDetail, AppError>;

    /// Channels in a guild that the bot can see.
    async fn list_channels(&self, guild_id: &GuildId) -> Result<Vec<ChannelSummary>, AppError>;

    /// Metadata for one channel.
    async fn get_channel(&self, channel_id: &ChannelId) -> Result<ChannelSummary, AppError>;

    /// A bounded page of channel history, newest first.
    async fn get_messages(
        &self,
        channel_id: &ChannelId,
        query: &MessageQuery,
    ) -> Result<Page<MessageSummary>, AppError>;

    /// One message by ID.
    async fn get_message(
        &self,
        channel_id: &ChannelId,
        message_id: &MessageId,
    ) -> Result<MessageSummary, AppError>;

    /// Guild members, ascending by user ID.
    ///
    /// Requires the privileged `GUILD_MEMBERS` intent; without it Discord
    /// answers `PERMISSION_DENIED`.
    async fn list_members(
        &self,
        guild_id: &GuildId,
        limit: u16,
        after: Option<&UserId>,
    ) -> Result<Page<MemberSummary>, AppError>;

    /// One guild member by user ID.
    async fn get_member(
        &self,
        guild_id: &GuildId,
        user_id: &UserId,
    ) -> Result<MemberSummary, AppError>;
}
