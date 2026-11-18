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
//! One method writes: `send_message`. It posts with mentions explicitly
//! disabled and without retries, because a retried `POST` may duplicate a
//! message. Nothing here can edit or delete.

use std::future::{Future, IntoFuture};
use std::time::Duration;

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use twilight_http::Client;
use twilight_http::response::ResponseFuture;
use twilight_http::response::marker::ListBody;
use twilight_model::channel::message::Message;
use twilight_model::channel::{Channel, ChannelType};
use twilight_model::guild::{Guild, Member};
use twilight_model::id::Id;
use twilight_model::id::marker::{ChannelMarker, GuildMarker, MessageMarker, UserMarker};
use twilight_model::user::CurrentUserGuild;

use crate::config::schema::{DiscordConfig, is_loopback_host};
use crate::discord::client::{DiscordApi, GUILDS_PAGE_MAX, MEMBERS_PAGE_MAX, MESSAGES_PAGE_MAX};
use crate::discord::errors::{map_discord_error, read_body_error};
use crate::discord::ids::{ChannelId, GuildId, MessageId, UserId};
use crate::discord::models::{
    ChannelKind, ChannelSummary, GuildDetail, GuildSummary, MemberSummary, MessageAuthor,
    MessageQuery, MessageSummary, Page,
};
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
        content: &str,
    ) -> Result<MessageSummary, AppError> {
        // Explicit, not implied: no parsed mentions, no role or user mentions,
        // no reply ping. Even though policy has already neutralised the text,
        // the request says so too. It is borrowed by the builder, so it lives
        // here rather than inside the closure.
        let allowed_mentions = twilight_model::channel::message::AllowedMentions::default();
        let message: Message = self
            .send_once("discord_send_message", || {
                read_one(IntoFuture::into_future(
                    self.client
                        .create_message(Id::new(channel_id.get()))
                        .content(content)
                        .allowed_mentions(Some(&allowed_mentions)),
                ))
            })
            .await?;

        Ok(message_summary(message))
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
        attachment_count: message.attachments.len(),
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
