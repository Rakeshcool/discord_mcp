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
//!
//! Writes add two things: the content itself is a policy input (length limit and
//! mention neutralisation, applied before the request is built), and every
//! attempt produces one audit record whatever its outcome.

use std::collections::VecDeque;
use std::fmt;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tracing::Instrument;

use crate::discord::client::{DiscordApi, MutationOutcome};
use crate::discord::ids::{ChannelId, GuildId, MessageId, UserId};
use crate::discord::models::{
    BotIdentity, ChannelSummary, GuildDetail, GuildSummary, MemberSummary, MessageQuery,
    MessageSummary, Page,
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
    /// Timestamps of recent mutations, oldest first, for the per-minute budget.
    /// A deque rather than a counter because the window rolls: an action a minute
    /// ago must stop counting without a background task to decrement anything.
    recent_actions: Arc<tokio::sync::Mutex<VecDeque<Instant>>>,
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
            recent_actions: Arc::new(tokio::sync::Mutex::new(VecDeque::new())),
        }
    }

    /// Consume one unit of the shared mutation budget.
    ///
    /// The budget is what makes an autonomous loop tolerable: a model that
    /// decides to clean a channel message by message runs out after
    /// `max_actions_per_minute` attempts instead of hammering the API.
    async fn consume_budget(&self, action: &'static str) -> Result<(), AppError> {
        let mut recent = self.recent_actions.lock().await;
        let now = Instant::now();
        while let Some(oldest) = recent.front() {
            if now.duration_since(*oldest) >= Duration::from_secs(60) {
                recent.pop_front();
            } else {
                break;
            }
        }
        let budget = usize::from(self.policy.action_budget());
        if recent.len() >= budget {
            return Err(AppError::policy_blocked(format!(
                "policy.autonomy.max_actions_per_minute ({budget}) is exhausted; `{action}` was not sent"
            )));
        }
        recent.push_back(now);
        Ok(())
    }

    /// The common tail of every mutation: dry-run short-circuit, dry-run policy,
    /// budget, then the audited request.
    ///
    /// A dry run never consumes budget and never records an attempt, because
    /// nothing was attempted; it still reports the policy decision, which is the
    /// point of asking.
    async fn run_mutation<T>(
        &self,
        action: &'static str,
        target: AuditTarget,
        dry_run: bool,
        body: impl Future<Output = Result<T, AppError>>,
    ) -> Result<MutationOutcome<T>, AppError> {
        if dry_run {
            return Ok(MutationOutcome::Planned);
        }
        if self.policy.requires_dry_run(action) {
            let error =
                AppError::policy_blocked(format!("`{action}` must be called with dry_run = true"));
            self.record_refusal(action, target, &error);
            return Err(error);
        }
        self.consume_budget(action).await.inspect_err(|error| {
            self.record_refusal(action, target, error);
        })?;
        self.run_write(action, target, body)
            .await
            .map(MutationOutcome::Applied)
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
        let correlation = crate::observability::correlation::current();
        let outcome = body
            .instrument(tracing::debug_span!("discord.call", action))
            .await;
        match outcome {
            Ok(value) => {
                self.audit
                    .record(&AuditEvent::allowed(action, target).with_correlation(correlation));
                Ok(value)
            }
            Err(error) => {
                if matches!(
                    error.category(),
                    ErrorCategory::PolicyBlocked | ErrorCategory::InvalidArgument
                ) {
                    self.audit.record(
                        &AuditEvent::denied(action, target, error.category())
                            .with_correlation(correlation),
                    );
                }
                Err(error)
            }
        }
    }

    /// Run `body`, recording exactly one outcome for a write attempt.
    ///
    /// A write must never be invisible in the audit trail, so unlike a read this
    /// records every outcome: the policy decision when policy decided, and a
    /// `Failed` record when the attempt got past policy and Discord rejected it.
    /// Exactly one record per attempt, so a count of `discord_send_message`
    /// records is a count of send attempts.
    ///
    /// Only `POLICY_BLOCKED` is a denial here. Policy's own `INVALID_ARGUMENT`
    /// refusals are recorded by the caller before reaching this point, so an
    /// `INVALID_ARGUMENT` that arrives here came from Discord (`400`/`422`) and
    /// is a failure, not a policy decision.
    async fn run_write<T>(
        &self,
        action: &'static str,
        target: AuditTarget,
        body: impl Future<Output = Result<T, AppError>>,
    ) -> Result<T, AppError> {
        let correlation = crate::observability::correlation::current();
        let outcome = body
            .instrument(tracing::debug_span!("discord.call", action))
            .await;
        match outcome {
            Ok(value) => {
                self.audit
                    .record(&AuditEvent::allowed(action, target).with_correlation(correlation));
                Ok(value)
            }
            Err(error) => {
                let event = match error.category() {
                    ErrorCategory::PolicyBlocked => {
                        AuditEvent::denied(action, target, error.category())
                    }
                    category => AuditEvent::failed(action, target, category),
                };
                self.audit.record(&event.with_correlation(correlation));
                Err(error)
            }
        }
    }

    /// Record a refusal of a write that policy decided before any request.
    fn record_refusal(&self, action: &'static str, target: AuditTarget, error: &AppError) {
        self.audit.record(
            &AuditEvent::denied(action, target, error.category())
                .with_correlation(crate::observability::correlation::current()),
        );
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

    async fn bot_identity(&self) -> Result<BotIdentity, AppError> {
        self.run("discord_get_capabilities", AuditTarget::none(), async {
            self.policy.check_read_enabled()?;
            let _permit = self.permit().await?;
            self.inner.bot_identity().await
        })
        .await
    }

    async fn send_message(
        &self,
        channel_id: &ChannelId,
        content: &str,
    ) -> Result<MessageSummary, AppError> {
        const ACTION: &str = "discord_send_message";
        let target = AuditTarget::for_channel(*channel_id);

        // Policy first, so its refusal is recorded as a denial and anything that
        // fails later is a failure. The write flag, length limit and mention
        // neutralisation are one decision; the allowlist is the next.
        let message = self
            .policy
            .prepare_outgoing_message(content)
            .inspect_err(|error| {
                self.record_refusal(ACTION, target, error);
            })?;
        self.policy.check_channel(channel_id).inspect_err(|error| {
            self.record_refusal(ACTION, target, error);
        })?;

        self.run_write(ACTION, target, async {
            let _permit = self.permit().await?;
            self.authorize_channel_guild(channel_id).await?;
            self.inner.send_message(channel_id, &message.content).await
        })
        .await
    }

    async fn edit_message(
        &self,
        channel_id: &ChannelId,
        message_id: &MessageId,
        content: &str,
        dry_run: bool,
    ) -> Result<MutationOutcome<MessageSummary>, AppError> {
        const ACTION: &str = "discord_edit_message";
        let target = AuditTarget::for_channel(*channel_id);
        // An edit passes the same content rules as a send, so it cannot be used
        // to smuggle in a mention or exceed the length limit.
        self.policy
            .check_tool(ACTION)
            .inspect_err(|error| self.record_refusal(ACTION, target, error))?;
        let message = self
            .policy
            .prepare_outgoing_message(content)
            .inspect_err(|error| self.record_refusal(ACTION, target, error))?;
        self.policy
            .check_channel(channel_id)
            .inspect_err(|error| self.record_refusal(ACTION, target, error))?;

        self.run_mutation(ACTION, target, dry_run, async {
            let _permit = self.permit().await?;
            self.authorize_channel_guild(channel_id).await?;
            let outcome = self
                .inner
                .edit_message(channel_id, message_id, &message.content, false)
                .await?;
            applied(outcome)
        })
        .await
    }

    async fn delete_message(
        &self,
        channel_id: &ChannelId,
        message_id: &MessageId,
        dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError> {
        const ACTION: &str = "discord_delete_message";
        let target = AuditTarget::for_channel(*channel_id);
        self.policy
            .check_tool(ACTION)
            .inspect_err(|error| self.record_refusal(ACTION, target, error))?;
        self.policy
            .check_channel(channel_id)
            .inspect_err(|error| self.record_refusal(ACTION, target, error))?;

        self.run_mutation(ACTION, target, dry_run, async {
            let _permit = self.permit().await?;
            self.authorize_channel_guild(channel_id).await?;
            let outcome = self
                .inner
                .delete_message(channel_id, message_id, false)
                .await?;
            applied(outcome)
        })
        .await
    }

    async fn bulk_delete_messages(
        &self,
        channel_id: &ChannelId,
        message_ids: &[MessageId],
        dry_run: bool,
    ) -> Result<MutationOutcome<u64>, AppError> {
        const ACTION: &str = "discord_bulk_delete_messages";
        let target = AuditTarget::for_channel(*channel_id);
        self.policy
            .check_tool(ACTION)
            .inspect_err(|error| self.record_refusal(ACTION, target, error))?;
        self.policy
            .check_channel(channel_id)
            .inspect_err(|error| self.record_refusal(ACTION, target, error))?;
        self.check_bulk_delete(message_ids)
            .inspect_err(|error| self.record_refusal(ACTION, target, error))?;

        self.run_mutation(ACTION, target, dry_run, async {
            let _permit = self.permit().await?;
            self.authorize_channel_guild(channel_id).await?;
            let outcome = self
                .inner
                .bulk_delete_messages(channel_id, message_ids, false)
                .await?;
            applied(outcome)
        })
        .await
    }

    async fn add_reaction(
        &self,
        channel_id: &ChannelId,
        message_id: &MessageId,
        emoji: &str,
        dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError> {
        self.message_mutation("discord_add_reaction", channel_id, dry_run, async move {
            applied(
                self.inner
                    .add_reaction(channel_id, message_id, emoji, false)
                    .await?,
            )
        })
        .await
    }

    async fn remove_reaction(
        &self,
        channel_id: &ChannelId,
        message_id: &MessageId,
        emoji: &str,
        dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError> {
        self.message_mutation("discord_remove_reaction", channel_id, dry_run, async move {
            applied(
                self.inner
                    .remove_reaction(channel_id, message_id, emoji, false)
                    .await?,
            )
        })
        .await
    }

    async fn clear_reactions(
        &self,
        channel_id: &ChannelId,
        message_id: &MessageId,
        dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError> {
        self.message_mutation("discord_clear_reactions", channel_id, dry_run, async move {
            applied(
                self.inner
                    .clear_reactions(channel_id, message_id, false)
                    .await?,
            )
        })
        .await
    }

    async fn pin_message(
        &self,
        channel_id: &ChannelId,
        message_id: &MessageId,
        dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError> {
        self.message_mutation("discord_pin_message", channel_id, dry_run, async move {
            applied(
                self.inner
                    .pin_message(channel_id, message_id, false)
                    .await?,
            )
        })
        .await
    }

    async fn unpin_message(
        &self,
        channel_id: &ChannelId,
        message_id: &MessageId,
        dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError> {
        self.message_mutation("discord_unpin_message", channel_id, dry_run, async move {
            applied(
                self.inner
                    .unpin_message(channel_id, message_id, false)
                    .await?,
            )
        })
        .await
    }
}

impl PolicyGate {
    /// Message-scoped mutations share one preamble: write flag, allowlist,
    /// concurrency permit, guild check when the guild allowlist narrows scope.
    async fn message_mutation(
        &self,
        action: &'static str,
        channel_id: &ChannelId,
        dry_run: bool,
        body: impl Future<Output = Result<(), AppError>>,
    ) -> Result<MutationOutcome<()>, AppError> {
        let target = AuditTarget::for_channel(*channel_id);
        self.policy
            .check_tool(action)
            .inspect_err(|error| self.record_refusal(action, target, error))?;
        self.policy
            .check_channel(channel_id)
            .inspect_err(|error| self.record_refusal(action, target, error))?;

        self.run_mutation(action, target, dry_run, async {
            let _permit = self.permit().await?;
            self.authorize_channel_guild(channel_id).await?;
            body.await
        })
        .await
    }

    /// Validate a bulk-delete batch before anything is sent.
    ///
    /// Discord caps a bulk delete at 100 messages and rejects anything older
    /// than two weeks, so both rules are checked here from the snowflake
    /// timestamp: no extra request, and a refusal names the offending message.
    fn check_bulk_delete(&self, message_ids: &[MessageId]) -> Result<(), AppError> {
        if message_ids.len() < 2 {
            return Err(AppError::invalid_argument(
                "`message_ids` must contain at least 2 messages",
            ));
        }
        let maximum = usize::from(self.policy.max_bulk_delete());
        if message_ids.len() > maximum {
            return Err(AppError::invalid_argument(format!(
                "`message_ids` has {} entries; policy.autonomy.max_bulk_delete is {maximum}",
                message_ids.len()
            )));
        }
        let mut seen = std::collections::BTreeSet::new();
        for id in message_ids {
            if !seen.insert(id.get()) {
                return Err(AppError::invalid_argument(format!(
                    "`message_ids` contains {id} more than once"
                )));
            }
        }
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis() as u64)
            .unwrap_or(0);
        for id in message_ids {
            let created_ms = snowflake_millis(*id);
            if now_ms.saturating_sub(created_ms) > BULK_DELETE_MAX_AGE_MS {
                return Err(AppError::invalid_argument(format!(
                    "message {id} is older than 14 days; Discord refuses to bulk-delete it, delete it individually instead"
                )));
            }
        }
        Ok(())
    }
}

/// Unwrap what the adapter did, treating a planned outcome as a bug.
///
/// Only the gate decides a mutation is a dry run; an implementation that
/// answers `Planned` on its own would mean policy had been skipped, so it is an
/// internal error rather than something to report as success.
fn applied<T>(outcome: MutationOutcome<T>) -> Result<T, AppError> {
    match outcome {
        MutationOutcome::Applied(value) => Ok(value),
        MutationOutcome::Planned => Err(AppError::internal(
            "the Discord adapter reported a planned mutation; only the policy gate may do that",
        )),
    }
}

/// Discord's epoch, in milliseconds (2015-01-01T00:00:00Z).
const DISCORD_EPOCH_MS: u64 = 1_420_070_400_000;

/// Two weeks, Discord's bulk-delete age ceiling.
const BULK_DELETE_MAX_AGE_MS: u64 = 14 * 24 * 60 * 60 * 1_000;

/// Creation time encoded in a snowflake: 42 bits of milliseconds since Discord's
/// epoch, followed by worker and sequence bits.
fn snowflake_millis(id: MessageId) -> u64 {
    (id.get() >> 22) + DISCORD_EPOCH_MS
}
