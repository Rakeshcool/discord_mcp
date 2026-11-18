//! The application-owned Discord interface.
//!
//! The application layer depends on this trait, never on twilight. That keeps
//! the adapter replaceable, makes the whole surface testable without a network,
//! and is what lets the policy layer sit between tool arguments and real
//! requests.
//!
//! Mutations are separate from the read surface on purpose. Every one of them
//! takes `dry_run`: when it is `true` the policy gate answers with
//! [`MutationOutcome::Planned`] without calling the implementation, so a caller
//! can validate a destructive intent and see its effect without a request. The
//! adapter itself always performs the operation; it never sees a `true` value.

use async_trait::async_trait;

use crate::discord::ids::{ChannelId, GuildId, MessageId, UserId};
use crate::discord::models::{
    BotIdentity, ChannelSummary, GuildDetail, GuildSummary, MemberSummary, MessageQuery,
    MessageSummary, Page,
};
use crate::error::AppError;

/// Discord's maximum page size for `GET /users/@me/guilds`.
pub const GUILDS_PAGE_MAX: u16 = 200;

/// Discord's maximum page size for channel messages.
pub const MESSAGES_PAGE_MAX: u16 = 100;

/// Discord's maximum page size for guild members.
pub const MEMBERS_PAGE_MAX: u16 = 1000;

/// What became of a mutation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MutationOutcome<T> {
    /// The request was made and Discord answered.
    Applied(T),
    /// `dry_run` was requested: policy accepted the intent and nothing was sent.
    Planned,
}

/// Discord operations, expressed in this crate's own types.
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

    /// The bot account behind the token.
    async fn bot_identity(&self) -> Result<BotIdentity, AppError>;

    /// Send one message to a channel.
    ///
    /// `content` has already passed policy: it is non-empty, within the configured
    /// length, and mention-neutralised. An implementation must still send it with
    /// mentions explicitly disabled, so a formatting change upstream can never
    /// turn into a notification.
    async fn send_message(
        &self,
        channel_id: &ChannelId,
        content: &str,
    ) -> Result<MessageSummary, AppError>;

    /// Replace a message's content. Mentions are neutralised by policy first.
    async fn edit_message(
        &self,
        channel_id: &ChannelId,
        message_id: &MessageId,
        content: &str,
        dry_run: bool,
    ) -> Result<MutationOutcome<MessageSummary>, AppError>;

    /// Delete one message.
    async fn delete_message(
        &self,
        channel_id: &ChannelId,
        message_id: &MessageId,
        dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError>;

    /// Delete between 2 and 100 messages, all younger than two weeks.
    async fn bulk_delete_messages(
        &self,
        channel_id: &ChannelId,
        message_ids: &[MessageId],
        dry_run: bool,
    ) -> Result<MutationOutcome<u64>, AppError>;

    /// Add the bot's reaction to a message.
    async fn add_reaction(
        &self,
        channel_id: &ChannelId,
        message_id: &MessageId,
        emoji: &str,
        dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError>;

    /// Remove the bot's own reaction from a message.
    async fn remove_reaction(
        &self,
        channel_id: &ChannelId,
        message_id: &MessageId,
        emoji: &str,
        dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError>;

    /// Remove every reaction from a message.
    async fn clear_reactions(
        &self,
        channel_id: &ChannelId,
        message_id: &MessageId,
        dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError>;

    /// Pin a message in its channel.
    async fn pin_message(
        &self,
        channel_id: &ChannelId,
        message_id: &MessageId,
        dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError>;

    /// Remove a pin.
    async fn unpin_message(
        &self,
        channel_id: &ChannelId,
        message_id: &MessageId,
        dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError>;
}
