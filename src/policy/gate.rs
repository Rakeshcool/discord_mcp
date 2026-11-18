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

use std::fmt;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tracing::Instrument;

use crate::discord::client::{DiscordApi, MutationOutcome};
use crate::discord::ids::{ChannelId, GuildId, MessageId, RoleId, UserId};
use crate::discord::models::{
    BotIdentity, ChannelEdit, ChannelSummary, GuildDetail, GuildSummary, InviteSummary,
    MemberSummary, MessageQuery, MessageSummary, NewChannel, NewInvite, NewRole, NewThread,
    OverwriteTarget, Page, RoleEdit, RoleSummary,
};
use crate::discord::permissions::PermissionBits;
use crate::error::{AppError, ErrorCategory};
use crate::policy::audit::{AuditEvent, AuditSink, AuditTarget};
use crate::policy::budget::ActionBudget;
use crate::policy::rules::Policy;

/// A Discord client with policy in front of it.
pub struct PolicyGate {
    inner: Arc<dyn DiscordApi>,
    policy: Arc<Policy>,
    permits: Arc<Semaphore>,
    audit: Arc<dyn AuditSink>,
    /// Recent mutation timestamps for the shared and per-channel windows. It is
    /// persisted when the gate was built with a state file, so a restart cannot
    /// reset the limit.
    budget: Arc<tokio::sync::Mutex<ActionBudget>>,
    /// The bot's user ID, resolved once per gate.
    ///
    /// It cannot change for the life of the process, and the role guard needs it
    /// on every edit, deletion and removal. The capability tool still reads the
    /// live identity, because the username can change.
    bot_user_id: tokio::sync::OnceCell<UserId>,
}

impl PolicyGate {
    /// Wrap `inner` in policy enforcement, recording decisions in `audit`.
    ///
    /// The budget lives for the process only. The application builds its gate
    /// with [`PolicyGate::with_budget_file`] so the window survives a restart.
    pub fn new(inner: Arc<dyn DiscordApi>, policy: Policy, audit: Arc<dyn AuditSink>) -> Self {
        Self::build(inner, policy, audit, ActionBudget::in_memory())
    }

    /// Wrap `inner` with a rolling budget persisted at `path`.
    ///
    /// An unreadable, malformed or unknown-version file is a startup error: a
    /// process that cannot see the window must not start with an empty one.
    pub fn with_budget_file(
        inner: Arc<dyn DiscordApi>,
        policy: Policy,
        audit: Arc<dyn AuditSink>,
        path: PathBuf,
    ) -> Result<Self, AppError> {
        Ok(Self::build(inner, policy, audit, ActionBudget::load(path)?))
    }

    fn build(
        inner: Arc<dyn DiscordApi>,
        policy: Policy,
        audit: Arc<dyn AuditSink>,
        budget: ActionBudget,
    ) -> Self {
        let permits = Arc::new(Semaphore::new(usize::from(
            policy.max_concurrent_requests(),
        )));
        Self {
            inner,
            policy: Arc::new(policy),
            permits,
            audit,
            budget: Arc::new(tokio::sync::Mutex::new(budget)),
            bot_user_id: tokio::sync::OnceCell::new(),
        }
    }

    /// Consume one unit of the shared mutation budget.
    ///
    /// The budget is what makes an autonomous loop tolerable: a model that
    /// decides to clean a channel message by message runs out after
    /// `max_actions_per_minute` attempts instead of hammering the API.
    async fn consume_budget(&self, action: &'static str) -> Result<(), AppError> {
        self.budget
            .lock()
            .await
            .consume_action(self.policy.action_budget(), action)
    }

    /// Consume one shared action and one send from `channel_id`'s own window.
    ///
    /// Sending is the mutation an autonomous loop reaches for most often, so it
    /// is bounded twice: by the shared budget every mutation pays into and by a
    /// per-channel cap that stops one busy channel from absorbing the whole
    /// allowance.
    async fn consume_send_budget(
        &self,
        channel_id: &ChannelId,
        action: &'static str,
    ) -> Result<(), AppError> {
        self.budget.lock().await.consume_send(
            self.policy.action_budget(),
            self.policy.send_budget(),
            *channel_id,
            action,
        )
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
    /// The bot's user ID, fetched at most once per gate.
    async fn bot_user_id(&self) -> Result<UserId, AppError> {
        self.bot_user_id
            .get_or_try_init(|| async { self.inner.bot_identity().await.map(|bot| bot.id) })
            .await
            .copied()
    }

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
        self.consume_send_budget(channel_id, ACTION)
            .await
            .inspect_err(|error| {
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

    async fn create_channel(
        &self,
        guild_id: &GuildId,
        spec: &NewChannel,
        dry_run: bool,
    ) -> Result<MutationOutcome<ChannelSummary>, AppError> {
        const ACTION: &str = "discord_create_channel";
        let target = AuditTarget::for_guild(*guild_id);
        self.policy
            .check_tool(ACTION)
            .inspect_err(|error| self.record_refusal(ACTION, target, error))?;
        self.policy
            .check_guild(guild_id)
            .inspect_err(|error| self.record_refusal(ACTION, target, error))?;
        // A nested channel is only created where the allowlist already reaches,
        // so a category outside scope cannot be used as a foothold.
        if let Some(parent) = spec.parent_id {
            self.policy
                .check_channel(&parent)
                .inspect_err(|error| self.record_refusal(ACTION, target, error))?;
        }

        self.run_mutation(ACTION, target, dry_run, async {
            let _permit = self.permit().await?;
            let outcome = self.inner.create_channel(guild_id, spec, false).await?;
            applied(outcome)
        })
        .await
    }

    async fn edit_channel(
        &self,
        channel_id: &ChannelId,
        spec: &ChannelEdit,
        dry_run: bool,
    ) -> Result<MutationOutcome<ChannelSummary>, AppError> {
        self.channel_mutation(
            "discord_edit_channel",
            channel_id,
            dry_run,
            true,
            async move {
                let outcome = self.inner.edit_channel(channel_id, spec, false).await?;
                applied(outcome)
            },
        )
        .await
    }

    async fn delete_channel(
        &self,
        channel_id: &ChannelId,
        dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError> {
        self.channel_mutation(
            "discord_delete_channel",
            channel_id,
            dry_run,
            true,
            async move {
                let outcome = self.inner.delete_channel(channel_id, false).await?;
                applied(outcome)
            },
        )
        .await
    }

    async fn set_permission_overwrite(
        &self,
        channel_id: &ChannelId,
        target: OverwriteTarget,
        allow: PermissionBits,
        deny: PermissionBits,
        dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError> {
        self.channel_mutation(
            "discord_set_permission_overwrite",
            channel_id,
            dry_run,
            true,
            async move {
                let outcome = self
                    .inner
                    .set_permission_overwrite(channel_id, target, allow, deny, false)
                    .await?;
                applied(outcome)
            },
        )
        .await
    }

    async fn delete_permission_overwrite(
        &self,
        channel_id: &ChannelId,
        target: OverwriteTarget,
        dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError> {
        self.channel_mutation(
            "discord_delete_permission_overwrite",
            channel_id,
            dry_run,
            true,
            async move {
                let outcome = self
                    .inner
                    .delete_permission_overwrite(channel_id, target, false)
                    .await?;
                applied(outcome)
            },
        )
        .await
    }

    async fn create_thread(
        &self,
        channel_id: &ChannelId,
        spec: &NewThread,
        dry_run: bool,
    ) -> Result<MutationOutcome<ChannelSummary>, AppError> {
        self.channel_mutation(
            "discord_create_thread",
            channel_id,
            dry_run,
            false,
            async move {
                let outcome = self.inner.create_thread(channel_id, spec, false).await?;
                applied(outcome)
            },
        )
        .await
    }

    async fn edit_thread(
        &self,
        thread_id: &ChannelId,
        archived: Option<bool>,
        locked: Option<bool>,
        dry_run: bool,
    ) -> Result<MutationOutcome<ChannelSummary>, AppError> {
        const ACTION: &str = "discord_edit_thread";
        let target = AuditTarget::for_channel(*thread_id);
        self.policy
            .check_tool(ACTION)
            .inspect_err(|error| self.record_refusal(ACTION, target, error))?;
        self.policy
            .check_protected_channel(thread_id)
            .inspect_err(|error| self.record_refusal(ACTION, target, error))?;
        // A thread is never on the allowlist when it was just created, so the
        // parent channel's permission stands in for it. That lookup costs a read,
        // so it waits for an attempt that policy would allow: a dry run reports
        // the local decision only, exactly as a channel tool does when the guild
        // allowlist needs the same lookup.
        if !dry_run && !self.policy.requires_dry_run(ACTION) {
            self.authorize_thread(thread_id)
                .await
                .inspect_err(|error| self.record_refusal(ACTION, target, error))?;
        }

        self.run_mutation(ACTION, target, dry_run, async {
            let _permit = self.permit().await?;
            let outcome = self
                .inner
                .edit_thread(thread_id, archived, locked, false)
                .await?;
            applied(outcome)
        })
        .await
    }

    async fn create_invite(
        &self,
        channel_id: &ChannelId,
        spec: &NewInvite,
        dry_run: bool,
    ) -> Result<MutationOutcome<InviteSummary>, AppError> {
        self.channel_mutation(
            "discord_create_invite",
            channel_id,
            dry_run,
            false,
            async move {
                let outcome = self.inner.create_invite(channel_id, spec, false).await?;
                applied(outcome)
            },
        )
        .await
    }

    async fn delete_invite(
        &self,
        code: &str,
        dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError> {
        const ACTION: &str = "discord_delete_invite";
        // Until the code is resolved there is no channel to name, so a refusal
        // before that point is recorded without a target.
        let mut target = AuditTarget::none();
        self.policy
            .check_tool(ACTION)
            .inspect_err(|error| self.record_refusal(ACTION, target, error))?;
        // The code does not name a channel, so policy resolves it before the
        // budget is spent: a code outside the allowlist never costs an action.
        // That read waits for a real attempt so a dry run sends nothing.
        if !dry_run && !self.policy.requires_dry_run(ACTION) {
            let channel_id = self
                .authorize_invite(code)
                .await
                .inspect_err(|error| self.record_refusal(ACTION, target, error))?;
            target = AuditTarget::for_channel(channel_id);
        }

        self.run_mutation(ACTION, target, dry_run, async {
            let _permit = self.permit().await?;
            let outcome = self.inner.delete_invite(code, false).await?;
            applied(outcome)
        })
        .await
    }

    /// The gate's own invite read.
    ///
    /// No tool exposes it, but it is reachable through `DiscordApi`, so it
    /// honours read policy and leaves one audited read rather than being a way
    /// around both.
    async fn get_invite(&self, code: &str) -> Result<InviteSummary, AppError> {
        self.run("discord_resolve_invite", AuditTarget::none(), async {
            self.policy.check_read_enabled()?;
            self.resolve_invite(code).await
        })
        .await
    }

    async fn create_role(
        &self,
        guild_id: &GuildId,
        spec: &NewRole,
        dry_run: bool,
    ) -> Result<MutationOutcome<RoleSummary>, AppError> {
        self.guild_mutation("discord_create_role", guild_id, dry_run, async move {
            let outcome = self.inner.create_role(guild_id, spec, false).await?;
            applied(outcome)
        })
        .await
    }

    async fn edit_role(
        &self,
        guild_id: &GuildId,
        role_id: &RoleId,
        spec: &RoleEdit,
        dry_run: bool,
    ) -> Result<MutationOutcome<RoleSummary>, AppError> {
        self.role_mutation(
            "discord_edit_role",
            guild_id,
            Some(role_id),
            dry_run,
            self.authorize_role_guard(guild_id, role_id, None),
            async move {
                let outcome = self.inner.edit_role(guild_id, role_id, spec, false).await?;
                applied(outcome)
            },
        )
        .await
    }

    async fn delete_role(
        &self,
        guild_id: &GuildId,
        role_id: &RoleId,
        dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError> {
        self.role_mutation(
            "discord_delete_role",
            guild_id,
            Some(role_id),
            dry_run,
            self.authorize_role_guard(guild_id, role_id, None),
            async move {
                let outcome = self.inner.delete_role(guild_id, role_id, false).await?;
                applied(outcome)
            },
        )
        .await
    }

    async fn assign_role(
        &self,
        guild_id: &GuildId,
        user_id: &UserId,
        role_id: &RoleId,
        dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError> {
        self.guild_mutation("discord_assign_role", guild_id, dry_run, async move {
            let outcome = self
                .inner
                .assign_role(guild_id, user_id, role_id, false)
                .await?;
            applied(outcome)
        })
        .await
    }

    async fn remove_role(
        &self,
        guild_id: &GuildId,
        user_id: &UserId,
        role_id: &RoleId,
        dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError> {
        self.role_mutation(
            "discord_remove_role",
            guild_id,
            None,
            dry_run,
            self.authorize_role_guard(guild_id, role_id, Some(user_id)),
            async move {
                let outcome = self
                    .inner
                    .remove_role(guild_id, user_id, role_id, false)
                    .await?;
                applied(outcome)
            },
        )
        .await
    }

    /// The gate's own role lookup: no tool exposes it, but `DiscordApi` does,
    /// so it honours read policy and guild scope and is audited.
    async fn highest_role_of(
        &self,
        guild_id: &GuildId,
        user_id: &UserId,
    ) -> Result<Option<RoleId>, AppError> {
        self.run(
            "discord_resolve_role",
            AuditTarget::for_guild(*guild_id),
            async {
                self.policy.check_read_enabled()?;
                self.policy.check_guild(guild_id)?;
                let _permit = self.permit().await?;
                self.inner.highest_role_of(guild_id, user_id).await
            },
        )
        .await
    }
}

impl PolicyGate {
    /// Guild-scoped mutations share one preamble: the administrative flag, the
    /// guild allowlist, the budget and the dry-run decision.
    async fn guild_mutation<T>(
        &self,
        action: &'static str,
        guild_id: &GuildId,
        dry_run: bool,
        body: impl Future<Output = Result<T, AppError>>,
    ) -> Result<MutationOutcome<T>, AppError> {
        let target = AuditTarget::for_guild(*guild_id);
        self.policy
            .check_tool(action)
            .inspect_err(|error| self.record_refusal(action, target, error))?;
        self.policy
            .check_guild(guild_id)
            .inspect_err(|error| self.record_refusal(action, target, error))?;

        self.run_mutation(action, target, dry_run, async {
            let _permit = self.permit().await?;
            body.await
        })
        .await
    }

    /// Role edits, deletions and removals additionally refuse to touch the bot's
    /// own highest role, and `protect` refuses an operator-protected role before
    /// the budget is spent.
    ///
    /// Changing or removing that role can strip the bot's permissions, so the
    /// guard is unconditional rather than a setting that a deployment could
    /// turn off and then lock itself out with. The lookups need requests, so
    /// the guard runs for a real attempt only; a dry run reports the local
    /// decision, as other remote-scope checks do.
    async fn role_mutation<T>(
        &self,
        action: &'static str,
        guild_id: &GuildId,
        protected_role: Option<&RoleId>,
        dry_run: bool,
        guard: impl Future<Output = Result<(), AppError>>,
        body: impl Future<Output = Result<T, AppError>>,
    ) -> Result<MutationOutcome<T>, AppError> {
        let target = AuditTarget::for_guild(*guild_id);
        self.policy
            .check_tool(action)
            .inspect_err(|error| self.record_refusal(action, target, error))?;
        self.policy
            .check_guild(guild_id)
            .inspect_err(|error| self.record_refusal(action, target, error))?;
        if let Some(role_id) = protected_role {
            self.policy
                .check_protected_role(role_id)
                .inspect_err(|error| self.record_refusal(action, target, error))?;
        }
        if !dry_run && !self.policy.requires_dry_run(action) {
            guard
                .await
                .inspect_err(|error| self.record_refusal(action, target, error))?;
        }

        self.run_mutation(action, target, dry_run, async {
            let _permit = self.permit().await?;
            body.await
        })
        .await
    }

    /// Refuse a change or removal aimed at the bot's own highest role.
    ///
    /// `user_id` is set for a member-role removal: taking a role from anyone
    /// else cannot lock the bot out, so the role list is only fetched when that
    /// member is the bot.
    async fn authorize_role_guard(
        &self,
        guild_id: &GuildId,
        role_id: &RoleId,
        user_id: Option<&UserId>,
    ) -> Result<(), AppError> {
        let _permit = self.permit().await?;
        let bot_id = self.bot_user_id().await?;
        if let Some(user_id) = user_id
            && bot_id != *user_id
        {
            return Ok(());
        }
        if self.inner.highest_role_of(guild_id, &bot_id).await? == Some(*role_id) {
            return Err(AppError::policy_blocked(format!(
                "role {role_id} is the bot's own highest role in guild {guild_id}; changing or removing it would remove the bot's access"
            )));
        }
        Ok(())
    }

    /// Resolve a thread's scope through its parent channel.
    async fn authorize_thread(&self, thread_id: &ChannelId) -> Result<(), AppError> {
        if self.policy.allows_channel(thread_id) {
            return self.authorize_channel_guild(thread_id).await;
        }
        // Resolution only; it must not spend an action or write an audit record
        // of its own, because the mutation it serves writes exactly one.
        let _permit = self.permit().await?;
        let channel = self.inner.get_channel(thread_id).await?;
        let parent = channel.parent_id.ok_or_else(|| {
            AppError::policy_blocked(
                "nothing is allowed here: neither the thread nor a parent channel is allowed",
            )
        })?;
        self.policy.check_channel(&parent)?;
        match channel.guild_id {
            Some(guild_id) => self.policy.check_guild(&guild_id)?,
            None if self.policy.guild_restricted() => {
                return Err(AppError::policy_blocked(format!(
                    "thread {thread_id} belongs to no guild, and policy.guild_allowlist is not empty"
                )));
            }
            None => {}
        }
        Ok(())
    }

    /// The lookup a mutation uses: policy-only, so it neither requires the read
    /// flag nor writes a record of its own. The mutation it scopes writes the one
    /// record for the call.
    async fn resolve_invite(&self, code: &str) -> Result<InviteSummary, AppError> {
        let _permit = self.permit().await?;
        self.inner.get_invite(code).await
    }

    /// Resolve an invite's channel before spending an action on it.
    async fn authorize_invite(&self, code: &str) -> Result<ChannelId, AppError> {
        let invite = self.resolve_invite(code).await?;
        let channel_id = invite.channel_id.ok_or_else(|| {
            AppError::policy_blocked("the invite does not name a channel policy can check")
        })?;
        self.policy.check_channel(&channel_id)?;
        match invite.guild_id {
            Some(guild_id) => self.policy.check_guild(&guild_id)?,
            None if self.policy.guild_restricted() => {
                return Err(AppError::policy_blocked(format!(
                    "invite {code} belongs to no guild, and policy.guild_allowlist is not empty"
                )));
            }
            None => {}
        }
        Ok(channel_id)
    }

    /// Channel-scoped structural mutations share one preamble: the control
    /// flag, the protected-channel guard, the channel allowlist, the budget and
    /// the guild check when the guild allowlist narrows scope.
    ///
    /// `protect` is set by tools that edit the channel itself. Protection is a
    /// hard local stop: it applies to an allowlisted channel too, and it is
    /// checked before the budget so a refusal costs no action.
    async fn channel_mutation<T>(
        &self,
        action: &'static str,
        channel_id: &ChannelId,
        dry_run: bool,
        protect: bool,
        body: impl Future<Output = Result<T, AppError>>,
    ) -> Result<MutationOutcome<T>, AppError> {
        let target = AuditTarget::for_channel(*channel_id);
        self.policy
            .check_tool(action)
            .inspect_err(|error| self.record_refusal(action, target, error))?;
        if protect {
            self.policy
                .check_protected_channel(channel_id)
                .inspect_err(|error| self.record_refusal(action, target, error))?;
        }
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
