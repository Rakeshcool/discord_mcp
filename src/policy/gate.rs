//! Enforcement: the only Discord handle the application hands out.
//!
//! [`PolicyGate`] wraps a [`DiscordApi`] and applies policy before delegating, so
//! a tool cannot reach Discord without passing the allowlists, the feature flags,
//! the page-size ceiling and the concurrency cap. Placement matters: this is a
//! decorator, not a step a caller could forget, and `AppState` stores the gated
//! handle rather than the raw client.
//!
//! Order per operation: feature flag, target allowlists, page limit, then a
//! concurrency permit, then — only when a guild allowlist narrows the scope —
//! resolving which guild a channel belongs to, then the request itself.

use std::fmt;
use std::future::Future;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::discord::client::DiscordApi;
use crate::discord::ids::{ChannelId, GuildId, MessageId, UserId};
use crate::discord::models::{
    ChannelSummary, GuildDetail, GuildSummary, MemberSummary, MessageQuery, MessageSummary, Page,
};
use crate::error::{AppError, ErrorCategory};
use crate::policy::audit::{AuditEvent, AuditSink, AuditTarget};
use crate::policy::rules::Policy;

/// A Discord client with policy in front of it.
pub struct PolicyGate {
    inner: Arc<dyn DiscordApi>,
    policy: Arc<Policy>,
    permits: Arc<Semaphore>,
    audit: Arc<dyn AuditSink>,
}

impl PolicyGate {
    /// Wrap `inner` in policy enforcement, recording decisions in `audit`.
    pub fn new(inner: Arc<dyn DiscordApi>, policy: Policy, audit: Arc<dyn AuditSink>) -> Self {
        let permits = Arc::new(Semaphore::new(usize::from(
            policy.max_concurrent_requests(),
        )));
        Self {
            inner,
            policy: Arc::new(policy),
            permits,
            audit,
        }
    }

    /// The policy being enforced.
    pub fn policy(&self) -> &Policy {
        &self.policy
    }

    /// Take one of the concurrency slots.
    ///
    /// Waiting is deliberate: the cap limits how many Discord requests are in
    /// flight, and queueing is the behaviour that keeps a burst within it. It
    /// cannot fail open — a waiter only proceeds when a slot frees up.
    async fn permit(&self) -> Result<OwnedSemaphorePermit, AppError> {
        Arc::clone(&self.permits)
            .acquire_owned()
            .await
            .map_err(|_| AppError::internal("the concurrency limiter is closed"))
    }

    /// Run `body`, recording the policy decision it reaches.
    async fn run<T>(
        &self,
        action: &'static str,
        target: AuditTarget,
        body: impl Future<Output = Result<T, AppError>>,
    ) -> Result<T, AppError> {
        match body.await {
            Ok(value) => {
                self.audit.record(&AuditEvent::allowed(action, target));
                Ok(value)
            }
            Err(error) => {
                if matches!(
                    error.category(),
                    ErrorCategory::PolicyBlocked | ErrorCategory::InvalidArgument
                ) {
                    self.audit
                        .record(&AuditEvent::denied(action, target, error.category()));
                }
                Err(error)
            }
        }
    }

    /// Check which guild a channel belongs to, when guilds are restricted.
    ///
    /// Costs one extra request only when `policy.guild_allowlist` is non-empty.
    /// A channel that belongs to no guild (a direct message) is refused under a
    /// guild allowlist: its guild cannot be shown to be allowed.
    async fn authorize_channel_guild(&self, channel_id: &ChannelId) -> Result<(), AppError> {
        if !self.policy.guild_restricted() {
            return Ok(());
        }
        let channel = self.inner.get_channel(channel_id).await?;
        match channel.guild_id {
            Some(guild_id) => self.policy.check_guild(&guild_id),
            None => Err(AppError::policy_blocked(format!(
                "channel {channel_id} belongs to no guild, and policy.guild_allowlist is not empty"
            ))),
        }
    }
}

impl fmt::Debug for PolicyGate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PolicyGate")
            .field("policy", &self.policy)
            .field("available_permits", &self.permits.available_permits())
            .field("audit", &self.audit)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl DiscordApi for PolicyGate {
    async fn list_guilds(
        &self,
        limit: u16,
        after: Option<&GuildId>,
    ) -> Result<Page<GuildSummary>, AppError> {
        self.run("discord_list_guilds", AuditTarget::none(), async {
            self.policy.check_read_enabled()?;
            let _permit = self.permit().await?;
            let mut page = self.inner.list_guilds(limit, after).await?;
            // A guild allowlist must not be a visibility leak: only allowlisted
            // guilds are returned. The cursor is left untouched so pagination
            // still advances through the guilds Discord returned; every page is
            // filtered the same way.
            page.items
                .retain(|guild| self.policy.allows_guild(&guild.id));
            Ok(page)
        })
        .await
    }

    async fn get_guild(&self, guild_id: &GuildId) -> Result<GuildDetail, AppError> {
        self.run(
            "discord_get_guild",
            AuditTarget::for_guild(*guild_id),
            async {
                self.policy.check_read_enabled()?;
                self.policy.check_guild(guild_id)?;
                let _permit = self.permit().await?;
                self.inner.get_guild(guild_id).await
            },
        )
        .await
    }

    async fn list_channels(&self, guild_id: &GuildId) -> Result<Vec<ChannelSummary>, AppError> {
        self.run(
            "discord_list_channels",
            AuditTarget::for_guild(*guild_id),
            async {
                self.policy.check_read_enabled()?;
                self.policy.check_guild(guild_id)?;
                let _permit = self.permit().await?;
                self.inner.list_channels(guild_id).await
            },
        )
        .await
    }

    async fn get_channel(&self, channel_id: &ChannelId) -> Result<ChannelSummary, AppError> {
        self.run(
            "discord_get_channel",
            AuditTarget::for_channel(*channel_id),
            async {
                self.policy.check_read_enabled()?;
                self.policy.check_channel(channel_id)?;
                let _permit = self.permit().await?;
                self.authorize_channel_guild(channel_id).await?;
                self.inner.get_channel(channel_id).await
            },
        )
        .await
    }

    async fn get_messages(
        &self,
        channel_id: &ChannelId,
        query: &MessageQuery,
    ) -> Result<Page<MessageSummary>, AppError> {
        self.run(
            "discord_get_messages",
            AuditTarget::for_channel(*channel_id),
            async {
                self.policy.check_read_enabled()?;
                self.policy.check_channel(channel_id)?;
                self.policy.check_page_limit(query.limit)?;
                let _permit = self.permit().await?;
                self.authorize_channel_guild(channel_id).await?;
                self.inner.get_messages(channel_id, query).await
            },
        )
        .await
    }

    async fn get_message(
        &self,
        channel_id: &ChannelId,
        message_id: &MessageId,
    ) -> Result<MessageSummary, AppError> {
        self.run(
            "discord_get_message",
            AuditTarget::for_channel(*channel_id),
            async {
                self.policy.check_read_enabled()?;
                self.policy.check_channel(channel_id)?;
                let _permit = self.permit().await?;
                self.authorize_channel_guild(channel_id).await?;
                self.inner.get_message(channel_id, message_id).await
            },
        )
        .await
    }

    async fn list_members(
        &self,
        guild_id: &GuildId,
        limit: u16,
        after: Option<&UserId>,
    ) -> Result<Page<MemberSummary>, AppError> {
        self.run(
            "discord_list_members",
            AuditTarget::for_guild(*guild_id),
            async {
                self.policy.check_read_enabled()?;
                self.policy.check_guild(guild_id)?;
                let _permit = self.permit().await?;
                self.inner.list_members(guild_id, limit, after).await
            },
        )
        .await
    }

    async fn get_member(
        &self,
        guild_id: &GuildId,
        user_id: &UserId,
    ) -> Result<MemberSummary, AppError> {
        self.run(
            "discord_get_member",
            AuditTarget::for_guild(*guild_id),
            async {
                self.policy.check_read_enabled()?;
                self.policy.check_guild(guild_id)?;
                let _permit = self.permit().await?;
                self.inner.get_member(guild_id, user_id).await
            },
        )
        .await
    }
}
