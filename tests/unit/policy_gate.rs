//! Policy enforcement, against a fake Discord client that records every call.
//!
//! "Denied" is asserted as *the fake was never called*, which is stronger than
//! checking an error message: it shows no request could have reached Discord.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::Semaphore;

use discord_mcp::config::schema::PolicyConfig;
use discord_mcp::discord::DiscordApi;
use discord_mcp::discord::ids::{ChannelId, GuildId, MessageId, UserId};
use discord_mcp::discord::models::{
    ChannelKind, ChannelSummary, GuildDetail, GuildSummary, MemberSummary, MessageAuthor,
    MessageQuery, MessageSummary, Page,
};
use discord_mcp::error::{AppError, ErrorCategory};
use discord_mcp::policy::audit::{AuditEvent, AuditOutcome, AuditSink, AuditTarget};
use discord_mcp::policy::gate::PolicyGate;
use discord_mcp::policy::rules::Policy;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Call {
    ListGuilds,
    GetGuild(u64),
    ListChannels(u64),
    GetChannel(u64),
    GetMessages(u64, u16),
    GetMessage(u64, u64),
    ListMembers(u64),
    GetMember(u64, u64),
}

#[derive(Debug)]
struct FakeDiscord {
    calls: Mutex<Vec<Call>>,
    channel_guilds: Mutex<HashMap<u64, Option<u64>>>,
    hold: Mutex<Option<Arc<Semaphore>>>,
    failure: Mutex<Option<AppError>>,
    guilds: Vec<GuildSummary>,
}

impl FakeDiscord {
    fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            channel_guilds: Mutex::new(HashMap::new()),
            hold: Mutex::new(None),
            failure: Mutex::new(None),
            guilds: Vec::new(),
        }
    }

    fn with_guilds(self, guilds: Vec<GuildSummary>) -> Self {
        Self { guilds, ..self }
    }

    fn with_channel(self, channel: u64, guild: Option<u64>) -> Self {
        self.channel_guilds
            .lock()
            .expect("channel map")
            .insert(channel, guild);
        self
    }

    fn failing(self, error: AppError) -> Self {
        *self.failure.lock().expect("failure") = Some(error);
        self
    }

    fn holding(self, semaphore: Arc<Semaphore>) -> Self {
        *self.hold.lock().expect("hold") = Some(semaphore);
        self
    }

    fn calls(&self) -> Vec<Call> {
        self.calls.lock().expect("calls").clone()
    }

    async fn record(&self, call: Call) -> Result<(), AppError> {
        self.calls.lock().expect("calls").push(call);
        let hold = self.hold.lock().expect("hold").clone();
        if let Some(semaphore) = hold {
            let _permit = semaphore.acquire().await.expect("hold permit");
        }
        match self.failure.lock().expect("failure").clone() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    fn guild_of(&self, channel: u64) -> Option<u64> {
        self.channel_guilds
            .lock()
            .expect("channel map")
            .get(&channel)
            .copied()
            .flatten()
    }
}

fn guild_summary(id: u64) -> GuildSummary {
    GuildSummary {
        id: GuildId::from_u64(id),
        name: format!("guild-{id}"),
        icon_hash: None,
        owner: false,
    }
}

fn channel_summary(id: u64, guild: Option<u64>) -> ChannelSummary {
    ChannelSummary {
        id: ChannelId::from_u64(id),
        guild_id: guild.map(GuildId::from_u64),
        name: Some(format!("channel-{id}")),
        kind: ChannelKind::Text,
        position: None,
        parent_id: None,
        topic: None,
        nsfw: false,
    }
}

fn message_summary(id: u64) -> MessageSummary {
    MessageSummary {
        id: MessageId::from_u64(id),
        channel_id: ChannelId::from_u64(4),
        guild_id: Some(GuildId::from_u64(1)),
        author: MessageAuthor {
            id: UserId::from_u64(7),
            username: "alice".to_string(),
            bot: false,
        },
        content: Some("hello".to_string()),
        timestamp: "2024-01-01T00:00:00.000000+00:00".to_string(),
        edited_timestamp: None,
        attachment_count: 0,
    }
}

fn member_summary(id: u64) -> MemberSummary {
    MemberSummary {
        user_id: UserId::from_u64(id),
        username: "alice".to_string(),
        nickname: None,
        roles: Vec::new(),
        joined_at: None,
    }
}

#[async_trait]
impl DiscordApi for FakeDiscord {
    async fn list_guilds(
        &self,
        _limit: u16,
        _after: Option<&GuildId>,
    ) -> Result<Page<GuildSummary>, AppError> {
        self.record(Call::ListGuilds).await?;
        Ok(Page::new(self.guilds.clone(), Some("cursor".to_string())))
    }

    async fn get_guild(&self, guild_id: &GuildId) -> Result<GuildDetail, AppError> {
        self.record(Call::GetGuild(guild_id.get())).await?;
        Ok(GuildDetail {
            id: *guild_id,
            name: format!("guild-{guild_id}"),
            icon_hash: None,
            description: None,
            owner_id: None,
            member_count: None,
        })
    }

    async fn list_channels(&self, guild_id: &GuildId) -> Result<Vec<ChannelSummary>, AppError> {
        self.record(Call::ListChannels(guild_id.get())).await?;
        Ok(vec![channel_summary(4, Some(guild_id.get()))])
    }

    async fn get_channel(&self, channel_id: &ChannelId) -> Result<ChannelSummary, AppError> {
        self.record(Call::GetChannel(channel_id.get())).await?;
        Ok(channel_summary(
            channel_id.get(),
            self.guild_of(channel_id.get()),
        ))
    }

    async fn get_messages(
        &self,
        channel_id: &ChannelId,
        query: &MessageQuery,
    ) -> Result<Page<MessageSummary>, AppError> {
        self.record(Call::GetMessages(channel_id.get(), query.limit))
            .await?;
        Ok(Page::new(
            vec![message_summary(9)],
            Some("9001".to_string()),
        ))
    }

    async fn get_message(
        &self,
        channel_id: &ChannelId,
        message_id: &MessageId,
    ) -> Result<MessageSummary, AppError> {
        self.record(Call::GetMessage(channel_id.get(), message_id.get()))
            .await?;
        Ok(message_summary(message_id.get()))
    }

    async fn list_members(
        &self,
        guild_id: &GuildId,
        limit: u16,
        _after: Option<&UserId>,
    ) -> Result<Page<MemberSummary>, AppError> {
        self.record(Call::ListMembers(guild_id.get())).await?;
        let _ = limit;
        Ok(Page::new(vec![member_summary(7)], None))
    }

    async fn get_member(
        &self,
        guild_id: &GuildId,
        user_id: &UserId,
    ) -> Result<MemberSummary, AppError> {
        self.record(Call::GetMember(guild_id.get(), user_id.get()))
            .await?;
        Ok(member_summary(user_id.get()))
    }
}

#[derive(Debug, Default)]
struct RecordingAudit {
    events: Mutex<Vec<AuditEvent>>,
}

impl AuditSink for RecordingAudit {
    fn record(&self, event: &AuditEvent) {
        self.events.lock().expect("audit").push(*event);
    }
}

impl RecordingAudit {
    fn events(&self) -> Vec<AuditEvent> {
        self.events.lock().expect("audit").clone()
    }
}

fn guild(id: u64) -> GuildId {
    GuildId::from_u64(id)
}

fn channel(id: u64) -> ChannelId {
    ChannelId::from_u64(id)
}

fn policy(config: PolicyConfig) -> Policy {
    Policy::from_config(&config).expect("the test configuration must produce a policy")
}

fn gate(fake: Arc<FakeDiscord>, policy: Policy, audit: Arc<RecordingAudit>) -> PolicyGate {
    PolicyGate::new(fake, policy, audit)
}

fn default_gate(fake: Arc<FakeDiscord>) -> PolicyGate {
    gate(
        fake,
        policy(PolicyConfig::default()),
        Arc::new(RecordingAudit::default()),
    )
}

#[tokio::test]
async fn read_disabled_blocks_every_operation() {
    let fake = Arc::new(FakeDiscord::new());
    let audit = Arc::new(RecordingAudit::default());
    let gated = gate(
        Arc::clone(&fake),
        policy(PolicyConfig {
            read_enabled: false,
            ..PolicyConfig::default()
        }),
        Arc::clone(&audit),
    );

    let errors = [
        gated.list_guilds(10, None).await.expect_err("list_guilds"),
        gated.get_guild(&guild(1)).await.expect_err("get_guild"),
        gated
            .list_channels(&guild(1))
            .await
            .expect_err("list_channels"),
        gated
            .get_channel(&channel(4))
            .await
            .expect_err("get_channel"),
        gated
            .get_messages(&channel(4), &MessageQuery::latest(10))
            .await
            .expect_err("get_messages"),
        gated
            .get_message(&channel(4), &MessageId::from_u64(9))
            .await
            .expect_err("get_message"),
        gated
            .list_members(&guild(1), 10, None)
            .await
            .expect_err("list_members"),
        gated
            .get_member(&guild(1), &UserId::from_u64(7))
            .await
            .expect_err("get_member"),
    ];

    for error in errors {
        assert_eq!(error.category(), ErrorCategory::PolicyBlocked, "{error}");
    }
    assert!(fake.calls().is_empty(), "no Discord call may be made");
    assert_eq!(audit.events().len(), 8, "every denial is audited");
    assert!(
        audit
            .events()
            .iter()
            .all(|event| event.outcome == AuditOutcome::Denied)
    );
}

#[tokio::test]
async fn a_guild_allowlist_blocks_unlisted_guilds() {
    let fake = Arc::new(FakeDiscord::new());
    let gated = gate(
        Arc::clone(&fake),
        policy(PolicyConfig {
            guild_allowlist: vec!["1".to_string()],
            ..PolicyConfig::default()
        }),
        Arc::new(RecordingAudit::default()),
    );

    let errors = [
        gated.get_guild(&guild(2)).await.expect_err("get_guild"),
        gated
            .list_channels(&guild(2))
            .await
            .expect_err("list_channels"),
        gated
            .list_members(&guild(2), 10, None)
            .await
            .expect_err("list_members"),
        gated
            .get_member(&guild(2), &UserId::from_u64(7))
            .await
            .expect_err("get_member"),
    ];

    for error in errors {
        assert_eq!(error.category(), ErrorCategory::PolicyBlocked, "{error}");
    }
    assert!(fake.calls().is_empty());
}

#[tokio::test]
async fn an_allowlisted_guild_is_served() {
    let fake = Arc::new(FakeDiscord::new());
    let gated = gate(
        Arc::clone(&fake),
        policy(PolicyConfig {
            guild_allowlist: vec!["1".to_string()],
            ..PolicyConfig::default()
        }),
        Arc::new(RecordingAudit::default()),
    );

    let detail = gated.get_guild(&guild(1)).await.expect("allowed");

    assert_eq!(detail.id, guild(1));
    assert_eq!(fake.calls(), vec![Call::GetGuild(1)]);
}

#[tokio::test]
async fn a_channel_allowlist_blocks_unlisted_channels() {
    let fake = Arc::new(FakeDiscord::new());
    let gated = gate(
        Arc::clone(&fake),
        policy(PolicyConfig {
            channel_allowlist: vec!["4".to_string()],
            ..PolicyConfig::default()
        }),
        Arc::new(RecordingAudit::default()),
    );

    let errors = [
        gated
            .get_channel(&channel(5))
            .await
            .expect_err("get_channel"),
        gated
            .get_messages(&channel(5), &MessageQuery::latest(10))
            .await
            .expect_err("get_messages"),
        gated
            .get_message(&channel(5), &MessageId::from_u64(9))
            .await
            .expect_err("get_message"),
    ];

    for error in errors {
        assert_eq!(error.category(), ErrorCategory::PolicyBlocked, "{error}");
    }
    assert!(fake.calls().is_empty());
}

#[tokio::test]
async fn a_restricted_channel_read_resolves_its_guild_first() {
    let fake = Arc::new(FakeDiscord::new().with_channel(4, Some(1)));
    let gated = gate(
        Arc::clone(&fake),
        policy(PolicyConfig {
            guild_allowlist: vec!["1".to_string()],
            ..PolicyConfig::default()
        }),
        Arc::new(RecordingAudit::default()),
    );

    gated
        .get_messages(&channel(4), &MessageQuery::latest(5))
        .await
        .expect("an allowlisted guild's channel is readable");

    assert_eq!(
        fake.calls(),
        vec![Call::GetChannel(4), Call::GetMessages(4, 5)],
        "the guild is resolved, then the request is made"
    );
}

#[tokio::test]
async fn a_channel_in_an_unlisted_guild_is_refused_before_the_target_call() {
    let fake = Arc::new(FakeDiscord::new().with_channel(4, Some(2)));
    let gated = gate(
        Arc::clone(&fake),
        policy(PolicyConfig {
            guild_allowlist: vec!["1".to_string()],
            ..PolicyConfig::default()
        }),
        Arc::new(RecordingAudit::default()),
    );

    let error = gated
        .get_messages(&channel(4), &MessageQuery::latest(5))
        .await
        .expect_err("guild 2 is not allowlisted");

    assert_eq!(error.category(), ErrorCategory::PolicyBlocked);
    assert_eq!(
        fake.calls(),
        vec![Call::GetChannel(4)],
        "the target request must never be made"
    );
}

#[tokio::test]
async fn a_channel_without_a_guild_is_refused_under_a_guild_allowlist() {
    let fake = Arc::new(FakeDiscord::new().with_channel(4, None));
    let gated = gate(
        Arc::clone(&fake),
        policy(PolicyConfig {
            guild_allowlist: vec!["1".to_string()],
            ..PolicyConfig::default()
        }),
        Arc::new(RecordingAudit::default()),
    );

    let error = gated
        .get_messages(&channel(4), &MessageQuery::latest(5))
        .await
        .expect_err("a channel with no guild cannot be shown to be allowed");

    assert_eq!(error.category(), ErrorCategory::PolicyBlocked);
    assert_eq!(fake.calls(), vec![Call::GetChannel(4)]);
}

#[tokio::test]
async fn listing_guilds_hides_guilds_outside_the_allowlist() {
    let fake = Arc::new(FakeDiscord::new().with_guilds(vec![guild_summary(1), guild_summary(2)]));
    let gated = gate(
        Arc::clone(&fake),
        policy(PolicyConfig {
            guild_allowlist: vec!["2".to_string()],
            ..PolicyConfig::default()
        }),
        Arc::new(RecordingAudit::default()),
    );

    let page = gated.list_guilds(10, None).await.expect("allowed");

    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].id, guild(2));
    assert_eq!(
        page.next_cursor.as_deref(),
        Some("cursor"),
        "the opaque cursor is untouched so pagination still advances"
    );
}

#[tokio::test]
async fn an_oversized_page_is_refused_without_a_request() {
    let fake = Arc::new(FakeDiscord::new());
    let gated = gate(
        Arc::clone(&fake),
        policy(PolicyConfig {
            max_messages_per_read: 10,
            ..PolicyConfig::default()
        }),
        Arc::new(RecordingAudit::default()),
    );

    let error = gated
        .get_messages(&channel(4), &MessageQuery::latest(11))
        .await
        .expect_err("above the policy ceiling");
    assert_eq!(error.category(), ErrorCategory::InvalidArgument);
    assert!(fake.calls().is_empty());

    gated
        .get_messages(&channel(4), &MessageQuery::latest(10))
        .await
        .expect("at the ceiling");
    assert_eq!(fake.calls(), vec![Call::GetMessages(4, 10)]);
}

#[tokio::test]
async fn a_transport_failure_is_not_remapped_and_not_audited_as_a_decision() {
    let fake = Arc::new(
        FakeDiscord::new().failing(AppError::new(ErrorCategory::RateLimited, "slow down")),
    );
    let audit = Arc::new(RecordingAudit::default());
    let gated = gate(
        Arc::clone(&fake),
        policy(PolicyConfig::default()),
        Arc::clone(&audit),
    );

    let error = gated.get_guild(&guild(1)).await.expect_err("inner failure");

    assert_eq!(error.category(), ErrorCategory::RateLimited);
    assert!(
        audit.events().is_empty(),
        "only policy decisions are audit records"
    );
}

#[tokio::test]
async fn decisions_are_audited_with_the_target_and_reason() {
    let fake = Arc::new(FakeDiscord::new());
    let audit = Arc::new(RecordingAudit::default());
    let gated = gate(
        Arc::clone(&fake),
        policy(PolicyConfig {
            guild_allowlist: vec!["1".to_string()],
            ..PolicyConfig::default()
        }),
        Arc::clone(&audit),
    );

    gated.get_guild(&guild(1)).await.expect("allowed");
    gated.get_guild(&guild(2)).await.expect_err("denied");

    let events = audit.events();
    assert_eq!(events.len(), 2);
    assert_eq!(
        events[0],
        AuditEvent::allowed("discord_get_guild", AuditTarget::for_guild(guild(1)))
    );
    assert_eq!(
        events[1],
        AuditEvent::denied(
            "discord_get_guild",
            AuditTarget::for_guild(guild(2)),
            ErrorCategory::PolicyBlocked
        )
    );
    assert_eq!(events[1].reason, Some(ErrorCategory::PolicyBlocked));
}

#[tokio::test]
async fn the_concurrency_cap_queues_instead_of_exceeding_itself() {
    let hold = Arc::new(Semaphore::new(0));
    let fake = Arc::new(FakeDiscord::new().holding(Arc::clone(&hold)));
    let gated = Arc::new(gate(
        Arc::clone(&fake),
        policy(PolicyConfig {
            max_concurrent_requests: 1,
            ..PolicyConfig::default()
        }),
        Arc::new(RecordingAudit::default()),
    ));

    let first = {
        let gated = Arc::clone(&gated);
        tokio::spawn(async move { gated.get_guild(&GuildId::from_u64(1)).await })
    };
    for _ in 0..200 {
        if !fake.calls().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(fake.calls(), vec![Call::GetGuild(1)], "the first call runs");

    let second = {
        let gated = Arc::clone(&gated);
        tokio::spawn(async move { gated.get_guild(&GuildId::from_u64(1)).await })
    };
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        fake.calls().len(),
        1,
        "the second call must wait for a concurrency slot"
    );

    hold.add_permits(2);
    assert!(first.await.expect("first task").is_ok());
    assert!(second.await.expect("second task").is_ok());
    assert_eq!(fake.calls(), vec![Call::GetGuild(1), Call::GetGuild(1)]);
}

#[tokio::test]
async fn the_gate_dispatches_through_a_dyn_discord_api() {
    let fake = Arc::new(FakeDiscord::new());
    let api: Arc<dyn DiscordApi> = Arc::new(default_gate(Arc::clone(&fake)));

    let detail = api.get_guild(&guild(1)).await.expect("allowed");

    assert_eq!(detail.id, guild(1));
}
