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

use crate::discord::ids::{ChannelId, GuildId, MessageId, RoleId, UserId};
use crate::discord::models::{
    BotIdentity, ChannelEdit, ChannelSummary, GuildDetail, GuildSummary, InviteSummary,
    MemberSummary, MessageQuery, MessageSummary, NewChannel, NewInvite, NewRole, NewThread,
    OverwriteTarget, Page, RoleEdit, RoleSummary,
};
use crate::discord::permissions::PermissionBits;
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

    /// Create a guild channel.
    async fn create_channel(
        &self,
        guild_id: &GuildId,
        spec: &NewChannel,
        dry_run: bool,
    ) -> Result<MutationOutcome<ChannelSummary>, AppError>;

    /// Change a channel's name, topic, age restriction or slowmode.
    async fn edit_channel(
        &self,
        channel_id: &ChannelId,
        spec: &ChannelEdit,
        dry_run: bool,
    ) -> Result<MutationOutcome<ChannelSummary>, AppError>;

    /// Delete a channel.
    async fn delete_channel(
        &self,
        channel_id: &ChannelId,
        dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError>;

    /// Set a role or member overwrite in a channel.
    async fn set_permission_overwrite(
        &self,
        channel_id: &ChannelId,
        target: OverwriteTarget,
        allow: PermissionBits,
        deny: PermissionBits,
        dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError>;

    /// Remove a role or member overwrite from a channel.
    async fn delete_permission_overwrite(
        &self,
        channel_id: &ChannelId,
        target: OverwriteTarget,
        dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError>;

    /// Create a thread, standalone or from a message.
    async fn create_thread(
        &self,
        channel_id: &ChannelId,
        spec: &NewThread,
        dry_run: bool,
    ) -> Result<MutationOutcome<ChannelSummary>, AppError>;

    /// Archive or unarchive a thread, optionally locking it.
    async fn edit_thread(
        &self,
        thread_id: &ChannelId,
        archived: Option<bool>,
        locked: Option<bool>,
        dry_run: bool,
    ) -> Result<MutationOutcome<ChannelSummary>, AppError>;

    /// Create an invite for a channel.
    async fn create_invite(
        &self,
        channel_id: &ChannelId,
        spec: &NewInvite,
        dry_run: bool,
    ) -> Result<MutationOutcome<InviteSummary>, AppError>;

    /// Revoke an invite by code.
    async fn delete_invite(
        &self,
        code: &str,
        dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError>;

    /// Look an invite up, so policy can find the channel it belongs to.
    async fn get_invite(&self, code: &str) -> Result<InviteSummary, AppError>;

    /// Create a guild role.
    async fn create_role(
        &self,
        guild_id: &GuildId,
        spec: &NewRole,
        dry_run: bool,
    ) -> Result<MutationOutcome<RoleSummary>, AppError>;

    /// Change a role's name, permissions, color or display flags.
    async fn edit_role(
        &self,
        guild_id: &GuildId,
        role_id: &RoleId,
        spec: &RoleEdit,
        dry_run: bool,
    ) -> Result<MutationOutcome<RoleSummary>, AppError>;

    /// Delete a role.
    async fn delete_role(
        &self,
        guild_id: &GuildId,
        role_id: &RoleId,
        dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError>;

    /// Give a member a role.
    async fn assign_role(
        &self,
        guild_id: &GuildId,
        user_id: &UserId,
        role_id: &RoleId,
        dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError>;

    /// Take a role away from a member.
    async fn remove_role(
        &self,
        guild_id: &GuildId,
        user_id: &UserId,
        role_id: &RoleId,
        dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError>;

    /// The bot's highest role in a guild, ignoring `@everyone`.
    ///
    /// `None` means the bot holds no role of its own, which is only possible in
    /// a guild where `@everyone` is its whole membership.
    async fn bot_highest_role(&self, guild_id: &GuildId) -> Result<Option<RoleId>, AppError>;
}
