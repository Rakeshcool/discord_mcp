//! twilight-http implementation of [`DiscordApi`].
//!
//! Responsibilities kept here and nowhere else:
//!
//! - building the client, including the endpoint decision (canonical Discord, or
//!   a local mock through twilight's proxy hook),
//! - per-request timeouts and bounded retries for transient failures,
//! - converting twilight models into this crate's domain types, so no twilight
//!   type escapes this module,
//! - clamping page sizes to Discord's maxima.
//!
//! Every mutation goes through `send_once`: one attempt, no retry, because a
//! repeated `POST`/`PATCH` after a timeout may apply twice. Reads use `send`,
//! which retries transient failures within the configured budget. Dry runs never
//! reach this module: the policy gate answers them itself.

use std::future::{Future, IntoFuture};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use twilight_http::Client;
use twilight_http::request::AuditLogReason;
use twilight_http::request::channel::reaction::RequestReactionType;
use twilight_http::response::ResponseFuture;
use twilight_http::response::marker::ListBody;
use twilight_model::channel::message::Message;
use twilight_model::channel::message::embed::{
    Embed, EmbedField, EmbedFooter, EmbedImage, EmbedThumbnail,
};
use twilight_model::channel::thread::AutoArchiveDuration;
use twilight_model::channel::{Channel, ChannelType};
use twilight_model::guild::invite::Invite;
use twilight_model::guild::{Guild, Member, Permissions, Role};
use twilight_model::http::attachment::Attachment;
use twilight_model::http::permission_overwrite::{PermissionOverwrite, PermissionOverwriteType};
use twilight_model::id::Id;
use twilight_model::id::marker::{
    ChannelMarker, GuildMarker, MessageMarker, RoleMarker, UserMarker,
};
use twilight_model::user::{CurrentUser, CurrentUserGuild};
use twilight_model::util::Timestamp;

use crate::config::schema::{DiscordConfig, is_loopback_host};
use crate::discord::client::{
    DiscordApi, GUILDS_PAGE_MAX, MEMBERS_PAGE_MAX, MESSAGES_PAGE_MAX, MutationOutcome,
};
use crate::discord::errors::{map_discord_error, read_body_error};
use crate::discord::ids::{ChannelId, GuildId, MessageId, RoleId, UserId};
use crate::discord::models::{
    BotIdentity, ChannelEdit, ChannelKind, ChannelSummary, GuildDetail, GuildSummary,
    InviteSummary, MemberSummary, MessageAttachment, MessageAuthor, MessageDraft, MessageEmbed,
    MessageEmbedField, MessageQuery, MessageReaction, MessageSummary, NewChannel, NewEmbed,
    NewInvite, NewRole, NewThread, OverwriteTarget, Page, RoleEdit, RoleSummary,
};
use crate::discord::permissions::PermissionBits;
use crate::discord::rate_limit::{backoff, is_transient};
use crate::error::{AppError, ErrorCategory};
use crate::security::secrets::BotToken;

/// Express the configured base URL as twilight proxy settings.
///
/// twilight builds `<scheme>://<host>/api/v10/<route>`, where the scheme and
/// host come from its proxy hook. A base URL is therefore a (host, scheme) pair
/// plus the fixed `/api` prefix, and that is exactly what a local mock server
/// needs. `None` means the canonical endpoint, which needs no proxy.
fn proxy_for(base: &str) -> Result<Option<(String, bool)>, AppError> {
    let base = base.trim();
    let (scheme, rest) = base
        .split_once("://")
        .ok_or_else(|| AppError::configuration("discord.api_base must be an absolute URL"))?;
    let insecure = match scheme {
        "https" => false,
        "http" => true,
        other => {
            return Err(AppError::configuration(format!(
                "discord.api_base must use http or https, not `{other}`"
            )));
        }
    };
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    if authority.is_empty() {
        return Err(AppError::configuration(
            "discord.api_base must include a host",
        ));
    }
    if path != "api" {
        return Err(AppError::configuration(format!(
            "discord.api_base must end with `/api`, because requests are built as `<base>/v10/<route>` (got `{base}`)"
        )));
    }
    if insecure && !is_loopback_host(authority.split(':').next().unwrap_or_default()) {
        return Err(AppError::configuration(format!(
            "discord.api_base may use plaintext http only for a loopback host (got `{authority}`)"
        )));
    }

    Ok((insecure || authority != "discord.com").then(|| (authority.to_string(), insecure)))
}

/// A Discord client backed by twilight-http.
pub struct TwilightDiscord {
    client: Client,
    timeout: Duration,
    max_retries: u32,
}

impl TwilightDiscord {
    /// Build the client from validated configuration and a bot token.
    pub fn new(config: &DiscordConfig, token: &BotToken) -> Result<Self, AppError> {
        let expected_version = format!("v{}", twilight_http::API_VERSION);
        if config.api_version.trim() != expected_version {
            return Err(AppError::configuration(format!(
                "discord.api_version must be `{expected_version}` to match the bundled Discord client"
            )));
        }
        let proxy = proxy_for(&config.api_base)?;
        install_crypto_provider();
        let timeout = Duration::from_secs(config.request_timeout_seconds);

        let mut builder = Client::builder()
            .token(token.expose().to_string())
            .timeout(timeout);
        if let Some((authority, insecure)) = proxy {
            builder = builder.proxy(authority, insecure);
        }

        Ok(Self {
            client: builder.build(),
            timeout,
            max_retries: config.max_retries,
        })
    }

    /// A read: attempts under one timeout each, retrying transient failures
    /// within the configured budget.
    async fn send<T, F>(
        &self,
        operation: &'static str,
        make: impl FnMut() -> F,
    ) -> Result<T, AppError>
    where
        F: Future<Output = Result<T, AttemptError>>,
    {
        self.attempt(operation, self.max_retries, make).await
    }

    /// A write: one attempt, no retry.
    ///
    /// Retrying a `POST` after a timeout, a reset or a `5xx` is ambiguous — the
    /// message may already exist — and a duplicate in a channel is worse than an
    /// error the caller can read and decide about. Rate limits are not affected:
    /// twilight's limiter waits out a `429` and resends, which is safe because
    /// Discord rejected that attempt.
    async fn send_once<T, F>(
        &self,
        operation: &'static str,
        make: impl FnMut() -> F,
    ) -> Result<T, AppError>
    where
        F: Future<Output = Result<T, AttemptError>>,
    {
        self.attempt(operation, 0, make).await
    }

    /// The one request path: attempts under one timeout each, retrying
    /// transient failures up to `retries` times.
    ///
    /// The timeout covers a whole attempt — rate-limit waits included — so a
    /// caller never waits longer than configured, whatever retry loop twilight
    /// runs internally.
    async fn attempt<T, F>(
        &self,
        operation: &'static str,
        retries: u32,
        mut make: impl FnMut() -> F,
    ) -> Result<T, AppError>
    where
        F: Future<Output = Result<T, AttemptError>>,
    {
        let mut attempt = 0u32;
        loop {
            let error = match tokio::time::timeout(self.timeout, make()).await {
                Err(_elapsed) => {
                    return Err(AppError::new(
                        ErrorCategory::Timeout,
                        format!(
                            "{operation} did not complete within {}s",
                            self.timeout.as_secs()
                        ),
                    ));
                }
                Ok(Ok(value)) => return Ok(value),
                // A body that could not be read is not retried: twilight cannot
                // tell a truncated body from a genuinely bad one, and a read
                // that already failed this way rarely succeeds unchanged.
                Ok(Err(AttemptError::Body)) => return Err(read_body_error(operation)),
                Ok(Err(AttemptError::Request(error))) => error,
            };

            if attempt >= retries || !is_transient(&error) {
                return Err(map_discord_error(operation, &error));
            }

            let delay = backoff(attempt);
            tracing::debug!(
                operation,
                attempt,
                delay_ms = delay.as_millis() as u64,
                "retrying a transient Discord failure"
            );
            tokio::time::sleep(delay).await;
            attempt += 1;
        }
    }
}

/// Select the process-wide rustls crypto provider, once.
///
/// twilight links rustls with `default-features = false`, so nothing else chooses
/// a provider and a TLS handshake would panic. Picking `ring` here (rather than
/// letting rustls guess) keeps the choice explicit and consistent everywhere.
fn install_crypto_provider() {
    use std::sync::Once;

    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        // Already installed by the embedding application: keep theirs.
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// Why a single attempt failed: the request, or the body it returned.
enum AttemptError {
    /// The request itself failed.
    Request(twilight_http::Error),
    /// The response arrived but its body could not be read as the expected model.
    Body,
}

/// Await a request and read its single JSON model.
async fn read_one<T>(future: ResponseFuture<T>) -> Result<T, AttemptError>
where
    T: DeserializeOwned + Unpin,
{
    let response = future.await.map_err(AttemptError::Request)?;
    response.model().await.map_err(|_| AttemptError::Body)
}

/// Await a request whose answer has no model the caller needs.
async fn read_empty<T: Unpin>(future: ResponseFuture<T>) -> Result<(), AttemptError> {
    future.await.map(|_| ()).map_err(AttemptError::Request)
}

/// Await a request and read its JSON array.
///
/// Separate from [`read_one`] only because twilight types a list response as
/// `Response<ListBody<T>>`: the model type differs, so one function cannot serve
/// both shapes.
async fn read_many<T>(future: ResponseFuture<ListBody<T>>) -> Result<Vec<T>, AttemptError>
where
    T: DeserializeOwned + Unpin,
{
    let response = future.await.map_err(AttemptError::Request)?;
    response.model().await.map_err(|_| AttemptError::Body)
}

impl std::fmt::Debug for TwilightDiscord {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TwilightDiscord")
            .field("timeout", &self.timeout)
            .field("max_retries", &self.max_retries)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl DiscordApi for TwilightDiscord {
    async fn list_guilds(
        &self,
        limit: u16,
        after: Option<&GuildId>,
    ) -> Result<Page<GuildSummary>, AppError> {
        let limit = clamp_limit("discord_list_guilds", limit, GUILDS_PAGE_MAX)?;
        let guilds: Vec<CurrentUserGuild> = self
            .send("discord_list_guilds", || {
                let mut request = self.client.current_user_guilds().limit(limit);
                if let Some(after) = after {
                    request = request.after(Id::new(after.get()));
                }
                read_many(IntoFuture::into_future(request))
            })
            .await?;

        let next_cursor = next_cursor(&guilds, limit, |guild| guild.id.get().to_string());
        Ok(Page::new(
            guilds
                .into_iter()
                .map(|guild| GuildSummary {
                    id: guild_id(guild.id),
                    name: guild.name,
                    icon_hash: guild.icon.map(|hash| hash.to_string()),
                    owner: guild.owner,
                })
                .collect(),
            next_cursor,
        ))
    }

    async fn get_guild(&self, guild_id: &GuildId) -> Result<GuildDetail, AppError> {
        let guild: Guild = self
            .send("discord_get_guild", || {
                read_one(IntoFuture::into_future(
                    self.client.guild(Id::new(guild_id.get())),
                ))
            })
            .await?;

        Ok(GuildDetail {
            id: self::guild_id(guild.id),
            name: guild.name,
            icon_hash: guild.icon.map(|hash| hash.to_string()),
            description: guild.description,
            owner_id: Some(self::user_id(guild.owner_id)),
            member_count: guild.member_count,
        })
    }

    async fn list_channels(&self, guild_id: &GuildId) -> Result<Vec<ChannelSummary>, AppError> {
        let channels: Vec<Channel> = self
            .send("discord_list_channels", || {
                read_many(IntoFuture::into_future(
                    self.client.guild_channels(Id::new(guild_id.get())),
                ))
            })
            .await?;

        Ok(channels.into_iter().map(channel_summary).collect())
    }

    async fn get_channel(&self, channel_id: &ChannelId) -> Result<ChannelSummary, AppError> {
        let channel: Channel = self
            .send("discord_get_channel", || {
                read_one(IntoFuture::into_future(
                    self.client.channel(Id::new(channel_id.get())),
                ))
            })
            .await?;

        Ok(channel_summary(channel))
    }

    async fn get_messages(
        &self,
        channel_id: &ChannelId,
        query: &MessageQuery,
    ) -> Result<Page<MessageSummary>, AppError> {
        let limit = clamp_limit("discord_get_messages", query.limit, MESSAGES_PAGE_MAX)?;
        if query.before.is_some() && query.after.is_some() {
            return Err(AppError::invalid_argument(
                "discord_get_messages: `before` and `after` are mutually exclusive",
            ));
        }
        let messages: Vec<Message> = self
            .send("discord_get_messages", || {
                let request = self.client.channel_messages(Id::new(channel_id.get()));
                read_many(match (query.before, query.after) {
                    (Some(before), _) => {
                        IntoFuture::into_future(request.before(Id::new(before.get())).limit(limit))
                    }
                    (None, Some(after)) => {
                        IntoFuture::into_future(request.after(Id::new(after.get())).limit(limit))
                    }
                    (None, None) => IntoFuture::into_future(request.limit(limit)),
                })
            })
            .await?;

        let next_cursor = next_cursor(&messages, limit, |message| message.id.get().to_string());
        Ok(Page::new(
            messages.into_iter().map(message_summary).collect(),
            next_cursor,
        ))
    }

    async fn get_message(
        &self,
        channel_id: &ChannelId,
        message_id: &MessageId,
    ) -> Result<MessageSummary, AppError> {
        let message: Message = self
            .send("discord_get_message", || {
                read_one(IntoFuture::into_future(
                    self.client
                        .message(Id::new(channel_id.get()), Id::new(message_id.get())),
                ))
            })
            .await?;

        Ok(message_summary(message))
    }

    async fn list_members(
        &self,
        guild_id: &GuildId,
        limit: u16,
        after: Option<&UserId>,
    ) -> Result<Page<MemberSummary>, AppError> {
        let limit = clamp_limit("discord_list_members", limit, MEMBERS_PAGE_MAX)?;
        let members: Vec<Member> = self
            .send("discord_list_members", || {
                let mut request = self
                    .client
                    .guild_members(Id::new(guild_id.get()))
                    .limit(limit);
                if let Some(after) = after {
                    request = request.after(Id::new(after.get()));
                }
                read_many(IntoFuture::into_future(request))
            })
            .await?;

        let next_cursor = next_cursor(&members, limit, |member| member.user.id.get().to_string());
        Ok(Page::new(
            members.into_iter().map(member_summary).collect(),
            next_cursor,
        ))
    }

    async fn get_member(
        &self,
        guild_id: &GuildId,
        user_id: &UserId,
    ) -> Result<MemberSummary, AppError> {
        let member: Member = self
            .send("discord_get_member", || {
                read_one(IntoFuture::into_future(
                    self.client
                        .guild_member(Id::new(guild_id.get()), Id::new(user_id.get())),
                ))
            })
            .await?;

        Ok(member_summary(member))
    }

    async fn send_message(
        &self,
        channel_id: &ChannelId,
        draft: &MessageDraft,
    ) -> Result<MessageSummary, AppError> {
        let mut attachments = Vec::with_capacity(draft.attachment_paths.len());
        for (index, path) in draft.attachment_paths.iter().enumerate() {
            let filename = path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| {
                    AppError::invalid_argument(format!("file {index} has no usable filename"))
                })?
                .to_string();
            let data = tokio::fs::read(path).await.map_err(|_| {
                AppError::invalid_argument(format!("file {index} could not be read"))
            })?;
            attachments.push(Attachment::from_bytes(
                filename,
                data,
                u64::try_from(index).unwrap_or(0) + 1,
            ));
        }
        let embeds: Vec<Embed> = draft.embeds.iter().map(new_embed).collect();

        // Explicit, not implied: no parsed mentions, no role or user mentions,
        // no reply ping. Even though policy has already neutralised the text,
        // the request says so too. These locals are borrowed by the builder, so
        // they live outside the closure.
        let allowed_mentions = twilight_model::channel::message::AllowedMentions::default();
        let content = draft.content.as_deref();
        let reply_to = draft.reply_to;
        let message: Message = self
            .send_once("discord_send_message", || {
                let mut request = self
                    .client
                    .create_message(Id::new(channel_id.get()))
                    .allowed_mentions(Some(&allowed_mentions));
                if let Some(content) = content {
                    request = request.content(content);
                }
                if !embeds.is_empty() {
                    request = request.embeds(&embeds);
                }
                if !attachments.is_empty() {
                    request = request.attachments(&attachments);
                }
                if let Some(reply_to) = reply_to {
                    request = request.reply(Id::new(reply_to.get()));
                }
                read_one(IntoFuture::into_future(request))
            })
            .await?;

        Ok(message_summary(message))
    }

    async fn bot_identity(&self) -> Result<BotIdentity, AppError> {
        let user: CurrentUser = self
            .send("discord_get_capabilities", || {
                read_one(IntoFuture::into_future(self.client.current_user()))
            })
            .await?;
        Ok(BotIdentity {
            id: user_id(user.id),
            username: user.name,
        })
    }

    async fn edit_message(
        &self,
        channel_id: &ChannelId,
        message_id: &MessageId,
        content: &str,
        _dry_run: bool,
    ) -> Result<MutationOutcome<MessageSummary>, AppError> {
        let message: Message = self
            .send_once("discord_edit_message", || {
                read_one(IntoFuture::into_future(
                    self.client
                        .update_message(Id::new(channel_id.get()), Id::new(message_id.get()))
                        .content(Some(content)),
                ))
            })
            .await?;
        Ok(MutationOutcome::Applied(message_summary(message)))
    }

    async fn delete_message(
        &self,
        channel_id: &ChannelId,
        message_id: &MessageId,
        _dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError> {
        self.send_once("discord_delete_message", || {
            read_empty(IntoFuture::into_future(self.client.delete_message(
                Id::new(channel_id.get()),
                Id::new(message_id.get()),
            )))
        })
        .await?;
        Ok(MutationOutcome::Applied(()))
    }

    async fn bulk_delete_messages(
        &self,
        channel_id: &ChannelId,
        message_ids: &[MessageId],
        _dry_run: bool,
    ) -> Result<MutationOutcome<u64>, AppError> {
        let ids: Vec<Id<MessageMarker>> = message_ids.iter().map(|id| Id::new(id.get())).collect();
        self.send_once("discord_bulk_delete_messages", || {
            read_empty(IntoFuture::into_future(
                self.client.delete_messages(Id::new(channel_id.get()), &ids),
            ))
        })
        .await?;
        Ok(MutationOutcome::Applied(ids.len() as u64))
    }

    async fn add_reaction(
        &self,
        channel_id: &ChannelId,
        message_id: &MessageId,
        emoji: &str,
        _dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError> {
        let emoji = reaction_type(emoji)?;
        self.send_once("discord_add_reaction", || {
            read_empty(IntoFuture::into_future(self.client.create_reaction(
                Id::new(channel_id.get()),
                Id::new(message_id.get()),
                &emoji,
            )))
        })
        .await?;
        Ok(MutationOutcome::Applied(()))
    }

    async fn remove_reaction(
        &self,
        channel_id: &ChannelId,
        message_id: &MessageId,
        emoji: &str,
        _dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError> {
        let emoji = reaction_type(emoji)?;
        self.send_once("discord_remove_reaction", || {
            read_empty(IntoFuture::into_future(
                self.client.delete_current_user_reaction(
                    Id::new(channel_id.get()),
                    Id::new(message_id.get()),
                    &emoji,
                ),
            ))
        })
        .await?;
        Ok(MutationOutcome::Applied(()))
    }

    async fn clear_reactions(
        &self,
        channel_id: &ChannelId,
        message_id: &MessageId,
        _dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError> {
        self.send_once("discord_clear_reactions", || {
            read_empty(IntoFuture::into_future(self.client.delete_all_reactions(
                Id::new(channel_id.get()),
                Id::new(message_id.get()),
            )))
        })
        .await?;
        Ok(MutationOutcome::Applied(()))
    }

    async fn pin_message(
        &self,
        channel_id: &ChannelId,
        message_id: &MessageId,
        _dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError> {
        self.send_once("discord_pin_message", || {
            read_empty(IntoFuture::into_future(
                self.client
                    .create_pin(Id::new(channel_id.get()), Id::new(message_id.get())),
            ))
        })
        .await?;
        Ok(MutationOutcome::Applied(()))
    }

    async fn unpin_message(
        &self,
        channel_id: &ChannelId,
        message_id: &MessageId,
        _dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError> {
        self.send_once("discord_unpin_message", || {
            read_empty(IntoFuture::into_future(
                self.client
                    .delete_pin(Id::new(channel_id.get()), Id::new(message_id.get())),
            ))
        })
        .await?;
        Ok(MutationOutcome::Applied(()))
    }

    async fn create_channel(
        &self,
        guild_id: &GuildId,
        spec: &NewChannel,
        _dry_run: bool,
    ) -> Result<MutationOutcome<ChannelSummary>, AppError> {
        let channel: Channel = self
            .send_once("discord_create_channel", || {
                let mut request = self
                    .client
                    .create_guild_channel(Id::new(guild_id.get()), &spec.name)
                    .kind(ChannelType::from(spec.kind.to_code()))
                    .nsfw(spec.nsfw);
                if let Some(parent) = spec.parent_id {
                    request = request.parent_id(Id::new(parent.get()));
                }
                if let Some(topic) = &spec.topic {
                    request = request.topic(topic);
                }
                read_one(IntoFuture::into_future(request))
            })
            .await?;
        Ok(MutationOutcome::Applied(channel_summary(channel)))
    }

    async fn edit_channel(
        &self,
        channel_id: &ChannelId,
        spec: &ChannelEdit,
        _dry_run: bool,
    ) -> Result<MutationOutcome<ChannelSummary>, AppError> {
        let channel: Channel = self
            .send_once("discord_edit_channel", || {
                let mut request = self.client.update_channel(Id::new(channel_id.get()));
                if let Some(name) = &spec.name {
                    request = request.name(name);
                }
                if let Some(topic) = &spec.topic {
                    request = request.topic(topic);
                }
                if let Some(nsfw) = spec.nsfw {
                    request = request.nsfw(nsfw);
                }
                if let Some(seconds) = spec.slowmode_seconds {
                    request = request.rate_limit_per_user(seconds);
                }
                read_one(IntoFuture::into_future(request))
            })
            .await?;
        Ok(MutationOutcome::Applied(channel_summary(channel)))
    }

    async fn delete_channel(
        &self,
        channel_id: &ChannelId,
        _dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError> {
        self.send_once("discord_delete_channel", || {
            read_one(IntoFuture::into_future(
                self.client.delete_channel(Id::new(channel_id.get())),
            ))
        })
        .await?;
        Ok(MutationOutcome::Applied(()))
    }

    async fn set_permission_overwrite(
        &self,
        channel_id: &ChannelId,
        target: OverwriteTarget,
        allow: PermissionBits,
        deny: PermissionBits,
        _dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError> {
        let overwrite = permission_overwrite(target, allow, deny);
        self.send_once("discord_set_permission_overwrite", || {
            read_empty(IntoFuture::into_future(
                self.client
                    .update_channel_permission(Id::new(channel_id.get()), &overwrite),
            ))
        })
        .await?;
        Ok(MutationOutcome::Applied(()))
    }

    async fn delete_permission_overwrite(
        &self,
        channel_id: &ChannelId,
        target: OverwriteTarget,
        _dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError> {
        self.send_once("discord_delete_permission_overwrite", || {
            let request = self
                .client
                .delete_channel_permission(Id::new(channel_id.get()));
            let request = match target {
                OverwriteTarget::Role(role) => request.role(Id::new(role.get())),
                OverwriteTarget::Member(user) => request.member(Id::new(user.get())),
            };
            read_empty(IntoFuture::into_future(request))
        })
        .await?;
        Ok(MutationOutcome::Applied(()))
    }

    async fn create_thread(
        &self,
        channel_id: &ChannelId,
        spec: &NewThread,
        _dry_run: bool,
    ) -> Result<MutationOutcome<ChannelSummary>, AppError> {
        let channel: Channel = self
            .send_once("discord_create_thread", || match spec.from_message {
                Some(message_id) => {
                    let mut request = self.client.create_thread_from_message(
                        Id::new(channel_id.get()),
                        Id::new(message_id.get()),
                        &spec.name,
                    );
                    if let Some(minutes) = spec.auto_archive_minutes {
                        request = request.auto_archive_duration(auto_archive(minutes));
                    }
                    read_one(IntoFuture::into_future(request))
                }
                None => {
                    let kind = if spec.private {
                        ChannelType::PrivateThread
                    } else {
                        ChannelType::PublicThread
                    };
                    let mut request =
                        self.client
                            .create_thread(Id::new(channel_id.get()), &spec.name, kind);
                    if let Some(minutes) = spec.auto_archive_minutes {
                        request = request.auto_archive_duration(auto_archive(minutes));
                    }
                    read_one(IntoFuture::into_future(request))
                }
            })
            .await?;
        Ok(MutationOutcome::Applied(channel_summary(channel)))
    }

    async fn edit_thread(
        &self,
        thread_id: &ChannelId,
        archived: Option<bool>,
        locked: Option<bool>,
        _dry_run: bool,
    ) -> Result<MutationOutcome<ChannelSummary>, AppError> {
        let channel: Channel = self
            .send_once("discord_edit_thread", || {
                let mut request = self.client.update_thread(Id::new(thread_id.get()));
                if let Some(archived) = archived {
                    request = request.archived(archived);
                }
                if let Some(locked) = locked {
                    request = request.locked(locked);
                }
                read_one(IntoFuture::into_future(request))
            })
            .await?;
        Ok(MutationOutcome::Applied(channel_summary(channel)))
    }

    async fn create_invite(
        &self,
        channel_id: &ChannelId,
        spec: &NewInvite,
        _dry_run: bool,
    ) -> Result<MutationOutcome<InviteSummary>, AppError> {
        let invite: Invite = self
            .send_once("discord_create_invite", || {
                read_one(IntoFuture::into_future(
                    self.client
                        .create_invite(Id::new(channel_id.get()))
                        .max_age(spec.max_age_seconds)
                        .max_uses(spec.max_uses)
                        .temporary(spec.temporary)
                        .unique(spec.unique),
                ))
            })
            .await?;
        Ok(MutationOutcome::Applied(invite_summary(invite)))
    }

    async fn delete_invite(
        &self,
        code: &str,
        _dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError> {
        self.send_once("discord_delete_invite", || {
            read_empty(IntoFuture::into_future(self.client.delete_invite(code)))
        })
        .await?;
        Ok(MutationOutcome::Applied(()))
    }

    async fn get_invite(&self, code: &str) -> Result<InviteSummary, AppError> {
        let invite: Invite = self
            .send("discord_resolve_invite", || {
                read_one(IntoFuture::into_future(self.client.invite(code)))
            })
            .await?;
        Ok(invite_summary(invite))
    }

    async fn create_role(
        &self,
        guild_id: &GuildId,
        spec: &NewRole,
        _dry_run: bool,
    ) -> Result<MutationOutcome<RoleSummary>, AppError> {
        let role: Role = self
            .send_once("discord_create_role", || {
                let mut request = self
                    .client
                    .create_role(Id::new(guild_id.get()))
                    .name(&spec.name)
                    .permissions(Permissions::from_bits_truncate(spec.permissions.bits()))
                    .hoist(spec.hoist)
                    .mentionable(spec.mentionable);
                if let Some(color) = spec.color {
                    request = request.color(color);
                }
                read_one(IntoFuture::into_future(request))
            })
            .await?;
        Ok(MutationOutcome::Applied(role_summary(role)))
    }

    async fn edit_role(
        &self,
        guild_id: &GuildId,
        role_id: &RoleId,
        spec: &RoleEdit,
        _dry_run: bool,
    ) -> Result<MutationOutcome<RoleSummary>, AppError> {
        let role: Role = self
            .send_once("discord_edit_role", || {
                let mut request = self
                    .client
                    .update_role(Id::new(guild_id.get()), Id::new(role_id.get()));
                if let Some(name) = &spec.name {
                    request = request.name(Some(name));
                }
                if let Some(permissions) = spec.permissions {
                    request =
                        request.permissions(Permissions::from_bits_truncate(permissions.bits()));
                }
                if let Some(color) = spec.color {
                    request = request.color(Some(color));
                }
                if let Some(hoist) = spec.hoist {
                    request = request.hoist(hoist);
                }
                if let Some(mentionable) = spec.mentionable {
                    request = request.mentionable(mentionable);
                }
                read_one(IntoFuture::into_future(request))
            })
            .await?;
        Ok(MutationOutcome::Applied(role_summary(role)))
    }

    async fn delete_role(
        &self,
        guild_id: &GuildId,
        role_id: &RoleId,
        _dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError> {
        self.send_once("discord_delete_role", || {
            read_empty(IntoFuture::into_future(
                self.client
                    .delete_role(Id::new(guild_id.get()), Id::new(role_id.get())),
            ))
        })
        .await?;
        Ok(MutationOutcome::Applied(()))
    }

    async fn assign_role(
        &self,
        guild_id: &GuildId,
        user_id: &UserId,
        role_id: &RoleId,
        _dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError> {
        self.send_once("discord_assign_role", || {
            read_empty(IntoFuture::into_future(self.client.add_guild_member_role(
                Id::new(guild_id.get()),
                Id::new(user_id.get()),
                Id::new(role_id.get()),
            )))
        })
        .await?;
        Ok(MutationOutcome::Applied(()))
    }

    async fn remove_role(
        &self,
        guild_id: &GuildId,
        user_id: &UserId,
        role_id: &RoleId,
        _dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError> {
        self.send_once("discord_remove_role", || {
            read_empty(IntoFuture::into_future(
                self.client.remove_guild_member_role(
                    Id::new(guild_id.get()),
                    Id::new(user_id.get()),
                    Id::new(role_id.get()),
                ),
            ))
        })
        .await?;
        Ok(MutationOutcome::Applied(()))
    }

    async fn set_member_nickname(
        &self,
        guild_id: &GuildId,
        user_id: &UserId,
        nickname: Option<&str>,
        reason: Option<&str>,
        _dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError> {
        let _member: Member = self
            .send_once("discord_set_member_nickname", || {
                let mut request = self
                    .client
                    .update_guild_member(Id::new(guild_id.get()), Id::new(user_id.get()))
                    .nick(nickname);
                if let Some(reason) = reason {
                    request = request.reason(reason);
                }
                read_one(IntoFuture::into_future(request))
            })
            .await?;
        Ok(MutationOutcome::Applied(()))
    }

    async fn kick_member(
        &self,
        guild_id: &GuildId,
        user_id: &UserId,
        reason: Option<&str>,
        _dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError> {
        self.send_once("discord_kick_member", || {
            let mut request = self
                .client
                .remove_guild_member(Id::new(guild_id.get()), Id::new(user_id.get()));
            if let Some(reason) = reason {
                request = request.reason(reason);
            }
            read_empty(IntoFuture::into_future(request))
        })
        .await?;
        Ok(MutationOutcome::Applied(()))
    }

    async fn ban_member(
        &self,
        guild_id: &GuildId,
        user_id: &UserId,
        delete_message_seconds: u32,
        reason: Option<&str>,
        _dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError> {
        self.send_once("discord_ban_member", || {
            let mut request = self
                .client
                .create_ban(Id::new(guild_id.get()), Id::new(user_id.get()))
                .delete_message_seconds(delete_message_seconds);
            if let Some(reason) = reason {
                request = request.reason(reason);
            }
            read_empty(IntoFuture::into_future(request))
        })
        .await?;
        Ok(MutationOutcome::Applied(()))
    }

    async fn unban_member(
        &self,
        guild_id: &GuildId,
        user_id: &UserId,
        reason: Option<&str>,
        _dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError> {
        self.send_once("discord_unban_member", || {
            let mut request = self
                .client
                .delete_ban(Id::new(guild_id.get()), Id::new(user_id.get()));
            if let Some(reason) = reason {
                request = request.reason(reason);
            }
            read_empty(IntoFuture::into_future(request))
        })
        .await?;
        Ok(MutationOutcome::Applied(()))
    }

    async fn timeout_member(
        &self,
        guild_id: &GuildId,
        user_id: &UserId,
        duration_seconds: u32,
        reason: Option<&str>,
        _dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError> {
        let until = timeout_until(duration_seconds)?;
        let _member: Member = self
            .send_once("discord_timeout_member", || {
                let mut request = self
                    .client
                    .update_guild_member(Id::new(guild_id.get()), Id::new(user_id.get()))
                    .communication_disabled_until(Some(until));
                if let Some(reason) = reason {
                    request = request.reason(reason);
                }
                read_one(IntoFuture::into_future(request))
            })
            .await?;
        Ok(MutationOutcome::Applied(()))
    }

    async fn clear_member_timeout(
        &self,
        guild_id: &GuildId,
        user_id: &UserId,
        reason: Option<&str>,
        _dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError> {
        let _member: Member = self
            .send_once("discord_clear_member_timeout", || {
                let mut request = self
                    .client
                    .update_guild_member(Id::new(guild_id.get()), Id::new(user_id.get()))
                    .communication_disabled_until(None);
                if let Some(reason) = reason {
                    request = request.reason(reason);
                }
                read_one(IntoFuture::into_future(request))
            })
            .await?;
        Ok(MutationOutcome::Applied(()))
    }

    async fn highest_role_of(
        &self,
        guild_id: &GuildId,
        user_id: &UserId,
    ) -> Result<Option<RoleId>, AppError> {
        let guild = Id::new(guild_id.get());
        let member: Member = self
            .send("discord_resolve_role", || {
                read_one(IntoFuture::into_future(
                    self.client.guild_member(guild, Id::new(user_id.get())),
                ))
            })
            .await?;
        let roles: Vec<Role> = self
            .send("discord_resolve_role", || {
                read_many(IntoFuture::into_future(self.client.roles(guild)))
            })
            .await?;

        Ok(roles
            .into_iter()
            .filter(|role| role.id.get() != guild_id.get() && member.roles.contains(&role.id))
            .max_by_key(|role| (role.position, role.id.get()))
            .map(|role| role_id(role.id)))
    }
}

/// Convert a timeout duration into Discord's absolute expiry timestamp.
fn timeout_until(duration_seconds: u32) -> Result<Timestamp, AppError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| AppError::internal("the system clock is before the Unix epoch"))?;
    let seconds = now
        .as_secs()
        .checked_add(u64::from(duration_seconds))
        .and_then(|value| i64::try_from(value).ok())
        .ok_or_else(|| AppError::invalid_argument("timeout duration is too large"))?;
    Timestamp::from_secs(seconds)
        .map_err(|_| AppError::invalid_argument("timeout duration is outside Discord's range"))
}

/// Turn a tool's emoji argument into a reaction target.
///
/// `name:id` is a custom emoji, anything else is a unicode glyph. Nothing is
/// inferred from the shape of the string beyond that: a malformed custom emoji
/// is an argument error, not a guess.
fn reaction_type(emoji: &str) -> Result<RequestReactionType<'_>, AppError> {
    let emoji = emoji.trim();
    if emoji.is_empty() {
        return Err(AppError::invalid_argument("`emoji` must not be empty"));
    }
    if let Some((name, id)) = emoji.split_once(':') {
        if name.is_empty() || id.is_empty() {
            return Err(AppError::invalid_argument(
                "`emoji` must be `name:id` for a custom emoji, or a unicode glyph",
            ));
        }
        let id = id.parse::<u64>().map_err(|_| {
            AppError::invalid_argument("`emoji` custom id must be a numeric emoji ID")
        })?;
        return Ok(RequestReactionType::Custom {
            id: Id::new(id),
            name: Some(name),
        });
    }
    if emoji.chars().count() > 32 {
        return Err(AppError::invalid_argument(
            "`emoji` is too long to be a unicode reaction",
        ));
    }
    Ok(RequestReactionType::Unicode { name: emoji })
}

/// Discord types invite counts as `u64`; the report keeps them small.
fn saturating_u16(value: u64) -> u16 {
    u16::try_from(value).unwrap_or(u16::MAX)
}

/// Map a validated auto-archive delay to twilight's enum.
///
/// The tool accepts only Discord's four documented values, so the fallback is
/// unreachable and exists to keep this total.
fn auto_archive(minutes: u16) -> AutoArchiveDuration {
    match minutes {
        60 => AutoArchiveDuration::Hour,
        1_440 => AutoArchiveDuration::Day,
        4_320 => AutoArchiveDuration::ThreeDays,
        10_080 => AutoArchiveDuration::Week,
        other => AutoArchiveDuration::Unknown { value: other },
    }
}

/// Build the overwrite body Discord expects.
///
/// An empty side is sent as zero rather than omitted: the overwrite is the
/// complete rule for that target, so an omitted half would mean "unchanged"
/// semantics the caller did not ask for.
fn permission_overwrite(
    target: OverwriteTarget,
    allow: PermissionBits,
    deny: PermissionBits,
) -> PermissionOverwrite {
    let (id, kind) = match target {
        OverwriteTarget::Role(role) => (role.get(), PermissionOverwriteType::Role),
        OverwriteTarget::Member(user) => (user.get(), PermissionOverwriteType::Member),
    };
    PermissionOverwrite {
        allow: Some(Permissions::from_bits_truncate(allow.bits())),
        deny: Some(Permissions::from_bits_truncate(deny.bits())),
        id: Id::new(id),
        kind,
    }
}

fn invite_summary(invite: Invite) -> InviteSummary {
    InviteSummary {
        code: invite.code,
        channel_id: invite.channel.map(|channel| channel.id).map(channel_id),
        guild_id: invite.guild.map(|guild| guild.id).map(guild_id),
        expires_at: invite.expires_at.map(|value| value.iso_8601().to_string()),
        max_uses: invite.max_uses.map(saturating_u16),
        uses: invite.uses.map(saturating_u16),
    }
}

/// Reject a zero page size and clamp anything above Discord's maximum.
fn clamp_limit(operation: &str, requested: u16, maximum: u16) -> Result<u16, AppError> {
    if requested == 0 {
        return Err(AppError::invalid_argument(format!(
            "{operation}: `limit` must be at least 1"
        )));
    }
    Ok(requested.min(maximum))
}

/// A cursor only exists when the page was full: a short page is the last page.
fn next_cursor<T>(items: &[T], limit: u16, id_of: impl Fn(&T) -> String) -> Option<String> {
    if items.len() < usize::from(limit) {
        return None;
    }
    items.last().map(id_of)
}

fn guild_id(id: Id<GuildMarker>) -> GuildId {
    GuildId::from_u64(id.get())
}

fn channel_id(id: Id<ChannelMarker>) -> ChannelId {
    ChannelId::from_u64(id.get())
}

fn message_id(id: Id<MessageMarker>) -> MessageId {
    MessageId::from_u64(id.get())
}

fn user_id(id: Id<UserMarker>) -> UserId {
    UserId::from_u64(id.get())
}

fn role_id(id: Id<RoleMarker>) -> RoleId {
    RoleId::from_u64(id.get())
}

fn channel_kind(kind: ChannelType) -> ChannelKind {
    // twilight's numeric channel type is Discord's, so `from_code` covers every
    // variant, named or unknown.
    ChannelKind::from_code(u8::from(kind))
}

fn channel_summary(channel: Channel) -> ChannelSummary {
    ChannelSummary {
        id: channel_id(channel.id),
        guild_id: channel.guild_id.map(guild_id),
        name: channel.name,
        kind: channel_kind(channel.kind),
        position: channel.position,
        parent_id: channel.parent_id.map(channel_id),
        topic: channel.topic,
        nsfw: channel.nsfw.unwrap_or(false),
    }
}

fn message_summary(message: Message) -> MessageSummary {
    let attachment_count = message.attachments.len();
    let embeds = message
        .embeds
        .into_iter()
        .take(10)
        .map(message_embed)
        .collect();
    let reactions = message
        .reactions
        .into_iter()
        .take(20)
        .map(|reaction| {
            let (emoji, emoji_id, animated) = match reaction.emoji {
                twilight_model::channel::message::EmojiReactionType::Unicode { name } => {
                    (name, None, false)
                }
                twilight_model::channel::message::EmojiReactionType::Custom {
                    animated,
                    id,
                    name,
                } => (
                    name.unwrap_or_else(|| id.get().to_string()),
                    Some(id.get().to_string()),
                    animated,
                ),
            };
            MessageReaction {
                emoji,
                emoji_id,
                animated,
                count: reaction.count,
                me: reaction.me,
            }
        })
        .collect();
    let attachments = message
        .attachments
        .into_iter()
        .take(10)
        .map(message_attachment)
        .collect();
    MessageSummary {
        id: message_id(message.id),
        channel_id: channel_id(message.channel_id),
        guild_id: message.guild_id.map(guild_id),
        author: MessageAuthor {
            id: user_id(message.author.id),
            username: message.author.name,
            bot: message.author.bot,
        },
        content: (!message.content.is_empty()).then_some(message.content),
        timestamp: message.timestamp.iso_8601().to_string(),
        edited_timestamp: message
            .edited_timestamp
            .map(|value| value.iso_8601().to_string()),
        attachment_count,
        embeds,
        reactions,
        attachments,
    }
}

fn message_embed(embed: Embed) -> MessageEmbed {
    MessageEmbed {
        kind: truncate_detail(embed.kind, 32),
        title: embed.title.map(|value| truncate_detail(value, 256)),
        description: embed.description.map(|value| truncate_detail(value, 4_096)),
        url: embed.url.map(|value| truncate_detail(value, 2_048)),
        color: embed.color,
        fields: embed
            .fields
            .into_iter()
            .take(25)
            .map(|field| MessageEmbedField {
                name: truncate_detail(field.name, 256),
                value: truncate_detail(field.value, 1_024),
                inline: field.inline,
            })
            .collect(),
        footer: embed
            .footer
            .map(|footer| truncate_detail(footer.text, 2_048)),
        image_url: embed.image.map(|image| truncate_detail(image.url, 2_048)),
        thumbnail_url: embed
            .thumbnail
            .map(|thumbnail| truncate_detail(thumbnail.url, 2_048)),
    }
}

fn message_attachment(attachment: twilight_model::channel::Attachment) -> MessageAttachment {
    MessageAttachment {
        id: attachment.id.get().to_string(),
        filename: truncate_detail(attachment.filename, 256),
        description: attachment
            .description
            .map(|value| truncate_detail(value, 1_024)),
        content_type: attachment
            .content_type
            .map(|value| truncate_detail(value, 128)),
        size: attachment.size,
        url: truncate_detail(attachment.url, 2_048),
        proxy_url: truncate_detail(attachment.proxy_url, 2_048),
        width: attachment.width,
        height: attachment.height,
    }
}

/// Cap a read detail without splitting a character. The ellipsis is part of the
/// cap, so the result never exceeds `max` characters.
fn truncate_detail(value: String, max: usize) -> String {
    if value.chars().count() <= max {
        return value;
    }
    let mut truncated: String = value.chars().take(max.saturating_sub(1)).collect();
    truncated.push('…');
    truncated
}

fn new_embed(embed: &NewEmbed) -> Embed {
    Embed {
        author: None,
        color: embed.color,
        description: embed.description.clone(),
        fields: embed
            .fields
            .iter()
            .map(|field| EmbedField {
                inline: field.inline,
                name: field.name.clone(),
                value: field.value.clone(),
            })
            .collect(),
        flags: None,
        footer: embed.footer.clone().map(|text| EmbedFooter {
            icon_url: None,
            proxy_icon_url: None,
            text,
        }),
        image: embed.image_url.clone().map(|url| EmbedImage {
            content_type: None,
            flags: None,
            height: None,
            placeholder: None,
            placeholder_version: None,
            proxy_url: None,
            url,
            width: None,
        }),
        kind: "rich".to_string(),
        provider: None,
        thumbnail: embed.thumbnail_url.clone().map(|url| EmbedThumbnail {
            height: None,
            proxy_url: None,
            url,
            width: None,
        }),
        timestamp: None,
        title: embed.title.clone(),
        url: embed.url.clone(),
        video: None,
    }
}

fn role_summary(role: Role) -> RoleSummary {
    RoleSummary {
        id: role_id(role.id),
        name: role.name,
        color: role.colors.primary_color,
        hoist: role.hoist,
        mentionable: role.mentionable,
        managed: role.managed,
        position: role.position,
        permissions: role.permissions.bits(),
    }
}

fn member_summary(member: Member) -> MemberSummary {
    MemberSummary {
        user_id: user_id(member.user.id),
        username: member.user.name,
        nickname: member.nick,
        roles: member
            .roles
            .into_iter()
            .map(|role| role.get().to_string())
            .collect(),
        joined_at: member.joined_at.map(|value| value.iso_8601().to_string()),
    }
}
