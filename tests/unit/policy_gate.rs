//! Policy enforcement, against a fake Discord client that records every call.
//!
//! "Denied" is asserted as *the fake was never called*, which is stronger than
//! checking an error message: it shows no request could have reached Discord.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::Semaphore;

use discord_mcp::config::schema::{AutonomyConfig, PolicyConfig};
use discord_mcp::discord::DiscordApi;
use discord_mcp::discord::client::MutationOutcome;
use discord_mcp::discord::ids::{ChannelId, GuildId, MessageId, UserId};
use discord_mcp::discord::models::{
    BotIdentity, ChannelEdit, ChannelKind, ChannelSummary, GuildDetail, GuildSummary,
    InviteSummary, MemberSummary, MessageAuthor, MessageQuery, MessageSummary, NewChannel,
    NewInvite, NewThread, OverwriteTarget, Page,
};
use discord_mcp::discord::permissions::PermissionBits;
use discord_mcp::error::{AppError, ErrorCategory};
use discord_mcp::policy::audit::{AuditEvent, AuditOutcome, AuditSink, AuditTarget};
use discord_mcp::policy::gate::PolicyGate;
use discord_mcp::policy::rules::{Policy, ToolRisk};

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
    SendMessage(u64, String),
    EditMessage(u64, u64, String),
    DeleteMessage(u64, u64),
    BulkDelete(u64, usize),
    AddReaction(u64, u64, String),
    RemoveReaction(u64, u64, String),
    ClearReactions(u64, u64),
    Pin(u64, u64),
    Unpin(u64, u64),
    CreateChannel(u64),
    EditChannel(u64),
    DeleteChannel(u64),
    SetOverwrite(u64, u64),
    DeleteOverwrite(u64, u64),
    CreateThread(u64),
    EditThread(u64),
    CreateInvite(u64),
    DeleteInvite(String),
    GetInvite(String),
    BotIdentity,
}

#[derive(Debug)]
struct FakeDiscord {
    calls: Mutex<Vec<Call>>,
    channel_guilds: Mutex<HashMap<u64, Option<u64>>>,
    parents: Mutex<HashMap<u64, u64>>,
    invite_channels: Mutex<HashMap<String, u64>>,
    hold: Mutex<Option<Arc<Semaphore>>>,
    failure: Mutex<Option<AppError>>,
    guilds: Vec<GuildSummary>,
}

impl FakeDiscord {
    fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            channel_guilds: Mutex::new(HashMap::new()),
            parents: Mutex::new(HashMap::new()),
            invite_channels: Mutex::new(HashMap::new()),
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

    fn with_parent(self, channel: u64, parent: u64) -> Self {
        self.parents
            .lock()
            .expect("parent map")
            .insert(channel, parent);
        self
    }

    fn with_invite(self, code: &str, channel: u64) -> Self {
        self.invite_channels
            .lock()
            .expect("invite map")
            .insert(code.to_string(), channel);
        self
    }

    fn parent_of(&self, channel: u64) -> Option<u64> {
        self.parents
            .lock()
            .expect("parent map")
            .get(&channel)
            .copied()
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
        let mut summary = channel_summary(channel_id.get(), self.guild_of(channel_id.get()));
        summary.parent_id = self.parent_of(channel_id.get()).map(ChannelId::from_u64);
        Ok(summary)
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

    async fn send_message(
        &self,
        channel_id: &ChannelId,
        content: &str,
    ) -> Result<MessageSummary, AppError> {
        self.record(Call::SendMessage(channel_id.get(), content.to_string()))
            .await?;
        Ok(message_summary(100))
    }

    async fn bot_identity(&self) -> Result<BotIdentity, AppError> {
        self.record(Call::BotIdentity).await?;
        Ok(BotIdentity {
            id: UserId::from_u64(99),
            username: "test-bot".to_string(),
        })
    }

    async fn edit_message(
        &self,
        channel_id: &ChannelId,
        message_id: &MessageId,
        content: &str,
        _dry_run: bool,
    ) -> Result<MutationOutcome<MessageSummary>, AppError> {
        self.record(Call::EditMessage(
            channel_id.get(),
            message_id.get(),
            content.to_string(),
        ))
        .await?;
        Ok(MutationOutcome::Applied(message_summary(message_id.get())))
    }

    async fn delete_message(
        &self,
        channel_id: &ChannelId,
        message_id: &MessageId,
        _dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError> {
        self.record(Call::DeleteMessage(channel_id.get(), message_id.get()))
            .await?;
        Ok(MutationOutcome::Applied(()))
    }

    async fn bulk_delete_messages(
        &self,
        channel_id: &ChannelId,
        message_ids: &[MessageId],
        _dry_run: bool,
    ) -> Result<MutationOutcome<u64>, AppError> {
        self.record(Call::BulkDelete(channel_id.get(), message_ids.len()))
            .await?;
        Ok(MutationOutcome::Applied(message_ids.len() as u64))
    }

    async fn add_reaction(
        &self,
        channel_id: &ChannelId,
        message_id: &MessageId,
        emoji: &str,
        _dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError> {
        self.record(Call::AddReaction(
            channel_id.get(),
            message_id.get(),
            emoji.to_string(),
        ))
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
        self.record(Call::RemoveReaction(
            channel_id.get(),
            message_id.get(),
            emoji.to_string(),
        ))
        .await?;
        Ok(MutationOutcome::Applied(()))
    }

    async fn clear_reactions(
        &self,
        channel_id: &ChannelId,
        message_id: &MessageId,
        _dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError> {
        self.record(Call::ClearReactions(channel_id.get(), message_id.get()))
            .await?;
        Ok(MutationOutcome::Applied(()))
    }

    async fn pin_message(
        &self,
        channel_id: &ChannelId,
        message_id: &MessageId,
        _dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError> {
        self.record(Call::Pin(channel_id.get(), message_id.get()))
            .await?;
        Ok(MutationOutcome::Applied(()))
    }

    async fn unpin_message(
        &self,
        channel_id: &ChannelId,
        message_id: &MessageId,
        _dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError> {
        self.record(Call::Unpin(channel_id.get(), message_id.get()))
            .await?;
        Ok(MutationOutcome::Applied(()))
    }

    async fn create_channel(
        &self,
        guild_id: &GuildId,
        _spec: &NewChannel,
        _dry_run: bool,
    ) -> Result<MutationOutcome<ChannelSummary>, AppError> {
        self.record(Call::CreateChannel(guild_id.get())).await?;
        Ok(MutationOutcome::Applied(channel_summary(
            40,
            Some(guild_id.get()),
        )))
    }

    async fn edit_channel(
        &self,
        channel_id: &ChannelId,
        _spec: &ChannelEdit,
        _dry_run: bool,
    ) -> Result<MutationOutcome<ChannelSummary>, AppError> {
        self.record(Call::EditChannel(channel_id.get())).await?;
        Ok(MutationOutcome::Applied(channel_summary(
            channel_id.get(),
            self.guild_of(channel_id.get()),
        )))
    }

    async fn delete_channel(
        &self,
        channel_id: &ChannelId,
        _dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError> {
        self.record(Call::DeleteChannel(channel_id.get())).await?;
        Ok(MutationOutcome::Applied(()))
    }

    async fn set_permission_overwrite(
        &self,
        channel_id: &ChannelId,
        target: OverwriteTarget,
        _allow: PermissionBits,
        _deny: PermissionBits,
        _dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError> {
        let target_id = match target {
            OverwriteTarget::Role(role) => role.get(),
            OverwriteTarget::Member(user) => user.get(),
        };
        self.record(Call::SetOverwrite(channel_id.get(), target_id))
            .await?;
        Ok(MutationOutcome::Applied(()))
    }

    async fn delete_permission_overwrite(
        &self,
        channel_id: &ChannelId,
        target: OverwriteTarget,
        _dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError> {
        let target_id = match target {
            OverwriteTarget::Role(role) => role.get(),
            OverwriteTarget::Member(user) => user.get(),
        };
        self.record(Call::DeleteOverwrite(channel_id.get(), target_id))
            .await?;
        Ok(MutationOutcome::Applied(()))
    }

    async fn create_thread(
        &self,
        channel_id: &ChannelId,
        _spec: &NewThread,
        _dry_run: bool,
    ) -> Result<MutationOutcome<ChannelSummary>, AppError> {
        self.record(Call::CreateThread(channel_id.get())).await?;
        Ok(MutationOutcome::Applied(channel_summary(
            41,
            self.guild_of(channel_id.get()),
        )))
    }

    async fn edit_thread(
        &self,
        thread_id: &ChannelId,
        _archived: Option<bool>,
        _locked: Option<bool>,
        _dry_run: bool,
    ) -> Result<MutationOutcome<ChannelSummary>, AppError> {
        self.record(Call::EditThread(thread_id.get())).await?;
        Ok(MutationOutcome::Applied(channel_summary(
            thread_id.get(),
            self.guild_of(thread_id.get()),
        )))
    }

    async fn create_invite(
        &self,
        channel_id: &ChannelId,
        _spec: &NewInvite,
        _dry_run: bool,
    ) -> Result<MutationOutcome<InviteSummary>, AppError> {
        self.record(Call::CreateInvite(channel_id.get())).await?;
        Ok(MutationOutcome::Applied(InviteSummary {
            code: "new-code".to_string(),
            channel_id: Some(*channel_id),
            guild_id: self.guild_of(channel_id.get()).map(GuildId::from_u64),
            expires_at: None,
            max_uses: None,
            uses: None,
        }))
    }

    async fn delete_invite(
        &self,
        code: &str,
        _dry_run: bool,
    ) -> Result<MutationOutcome<()>, AppError> {
        self.record(Call::DeleteInvite(code.to_string())).await?;
        Ok(MutationOutcome::Applied(()))
    }

    async fn get_invite(&self, code: &str) -> Result<InviteSummary, AppError> {
        self.record(Call::GetInvite(code.to_string())).await?;
        let channel_id = self
            .invite_channels
            .lock()
            .expect("invite map")
            .get(code)
            .copied();
        Ok(InviteSummary {
            code: code.to_string(),
            channel_id: channel_id.map(ChannelId::from_u64),
            guild_id: Some(guild(1)),
            expires_at: None,
            max_uses: None,
            uses: None,
        })
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

fn write_enabled(channels: &[&str]) -> Policy {
    policy(PolicyConfig {
        write_enabled: true,
        channel_allowlist: channels.iter().map(|id| id.to_string()).collect(),
        ..PolicyConfig::default()
    })
}

#[tokio::test]
async fn writes_are_refused_by_default_and_the_refusal_is_audited() {
    let fake = Arc::new(FakeDiscord::new());
    let audit = Arc::new(RecordingAudit::default());
    let gated = gate(
        Arc::clone(&fake),
        policy(PolicyConfig::default()),
        Arc::clone(&audit),
    );

    let error = gated
        .send_message(&channel(4), "hello")
        .await
        .expect_err("writes are off by default");

    assert_eq!(error.category(), ErrorCategory::PolicyBlocked);
    assert!(fake.calls().is_empty(), "no Discord call may be made");
    assert_eq!(
        audit.events(),
        vec![AuditEvent::denied(
            "discord_send_message",
            AuditTarget::for_channel(channel(4)),
            ErrorCategory::PolicyBlocked
        )]
    );
}

#[tokio::test]
async fn a_write_to_an_unlisted_channel_is_refused_before_any_call() {
    let fake = Arc::new(FakeDiscord::new());
    let gated = gate(
        Arc::clone(&fake),
        write_enabled(&["4"]),
        Arc::new(RecordingAudit::default()),
    );

    let error = gated
        .send_message(&channel(5), "hello")
        .await
        .expect_err("channel 5 is not on the allowlist");

    assert_eq!(error.category(), ErrorCategory::PolicyBlocked);
    assert!(fake.calls().is_empty());
}

#[tokio::test]
async fn an_allowlisted_write_sends_neutralised_content_and_is_audited() {
    let fake = Arc::new(FakeDiscord::new());
    let audit = Arc::new(RecordingAudit::default());
    let gated = gate(Arc::clone(&fake), write_enabled(&["4"]), Arc::clone(&audit));

    let sent = gated
        .send_message(&channel(4), "@everyone hi <@123>")
        .await
        .expect("an allowlisted channel may be written to");

    assert_eq!(sent.id, MessageId::from_u64(100));
    let calls = fake.calls();
    assert_eq!(calls.len(), 1);
    let Call::SendMessage(target, content) = &calls[0] else {
        panic!("expected one send, got {:?}", calls[0]);
    };
    assert_eq!(*target, 4);
    assert!(
        !content.contains("@everyone") && !content.contains("<@"),
        "a mention must not survive to the request: {content:?}"
    );
    assert!(
        content.contains("@\u{200b}everyone") && content.contains("<\u{200b}@123>"),
        "the mention stays readable but unparsed: {content:?}"
    );
    assert_eq!(
        audit.events(),
        vec![AuditEvent::allowed(
            "discord_send_message",
            AuditTarget::for_channel(channel(4))
        )]
    );
}

#[tokio::test]
async fn an_empty_or_oversized_write_is_refused_before_any_call() {
    let fake = Arc::new(FakeDiscord::new());
    let audit = Arc::new(RecordingAudit::default());
    let gated = gate(
        Arc::clone(&fake),
        policy(PolicyConfig {
            write_enabled: true,
            channel_allowlist: vec!["4".to_string()],
            max_message_characters: 10,
            ..PolicyConfig::default()
        }),
        Arc::clone(&audit),
    );

    let empty = gated
        .send_message(&channel(4), "   ")
        .await
        .expect_err("an empty body must be refused");
    let long = gated
        .send_message(&channel(4), "01234567890")
        .await
        .expect_err("eleven characters exceed the limit of ten");

    assert_eq!(empty.category(), ErrorCategory::InvalidArgument);
    assert_eq!(long.category(), ErrorCategory::InvalidArgument);
    assert!(fake.calls().is_empty());
    assert_eq!(
        audit.events().len(),
        2,
        "a refused attempt still produces a record"
    );
}

#[tokio::test]
async fn a_permission_failure_on_a_write_is_audited_as_failed() {
    let fake = Arc::new(FakeDiscord::new().failing(AppError::new(
        ErrorCategory::Permission,
        "discord_send_message failed: Missing Permissions",
    )));
    let audit = Arc::new(RecordingAudit::default());
    let gated = gate(Arc::clone(&fake), write_enabled(&["4"]), Arc::clone(&audit));

    let error = gated
        .send_message(&channel(4), "hello")
        .await
        .expect_err("Discord refused the send");

    assert_eq!(error.category(), ErrorCategory::Permission);
    let events = audit.events();
    assert_eq!(events.len(), 1, "one record per attempt");
    assert_eq!(events[0].outcome, AuditOutcome::Failed);
    assert_eq!(events[0].reason, Some(ErrorCategory::Permission));
}

#[tokio::test]
async fn a_discord_side_invalid_argument_is_a_failure_not_a_policy_denial() {
    let fake = Arc::new(FakeDiscord::new().failing(AppError::new(
        ErrorCategory::InvalidArgument,
        "discord_send_message failed: Discord returned 400: invalid message fields",
    )));
    let audit = Arc::new(RecordingAudit::default());
    let gated = gate(Arc::clone(&fake), write_enabled(&["4"]), Arc::clone(&audit));

    let error = gated
        .send_message(&channel(4), "hello")
        .await
        .expect_err("Discord rejected the request");

    assert_eq!(error.category(), ErrorCategory::InvalidArgument);
    assert_eq!(
        audit.events()[0].outcome,
        AuditOutcome::Failed,
        "policy allowed this attempt; Discord is what refused it"
    );
}

#[tokio::test]
async fn a_rate_limited_write_is_returned_unchanged_and_audited() {
    let fake = Arc::new(
        FakeDiscord::new().failing(AppError::new(ErrorCategory::RateLimited, "slow down")),
    );
    let audit = Arc::new(RecordingAudit::default());
    let gated = gate(Arc::clone(&fake), write_enabled(&["4"]), Arc::clone(&audit));

    let error = gated
        .send_message(&channel(4), "hello")
        .await
        .expect_err("the limiter is exhausted");

    assert_eq!(error.category(), ErrorCategory::RateLimited);
    assert_eq!(
        fake.calls(),
        vec![Call::SendMessage(4, "hello".to_string())],
        "the gate neither hides nor retries a rate limit"
    );
    assert_eq!(audit.events()[0].outcome, AuditOutcome::Failed);
    assert_eq!(audit.events()[0].reason, Some(ErrorCategory::RateLimited));
}

#[tokio::test]
async fn audit_records_carry_the_calling_tool_calls_correlation_id() {
    use discord_mcp::observability::correlation;

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

    let allowed = guild(1);
    correlation::scope_tool("discord_get_guild", gated.get_guild(&allowed))
        .await
        .expect("an allowlisted guild is readable");
    let denied = guild(2);
    correlation::scope_tool("discord_get_guild", gated.get_guild(&denied))
        .await
        .expect_err("guild 2 is not allowlisted");

    let events = audit.events();
    assert_eq!(events.len(), 2);
    let first = events[0]
        .correlation
        .expect("an allowed record must carry an id");
    let second = events[1]
        .correlation
        .expect("a denied record must carry an id");
    assert_ne!(first, second, "each tool call has its own id");
}

#[tokio::test]
async fn the_gate_dispatches_through_a_dyn_discord_api() {
    let fake = Arc::new(FakeDiscord::new());
    let api: Arc<dyn DiscordApi> = Arc::new(default_gate(Arc::clone(&fake)));

    let detail = api.get_guild(&guild(1)).await.expect("allowed");

    assert_eq!(detail.id, guild(1));
}

fn mutation_config() -> PolicyConfig {
    PolicyConfig {
        write_enabled: true,
        control_enabled: true,
        channel_allowlist: vec!["4".to_string()],
        ..PolicyConfig::default()
    }
}

#[test]
fn every_risk_class_is_refused_when_its_flag_is_off() {
    let policy = policy(PolicyConfig::default());

    for tool in [
        "discord_send_message",
        "discord_edit_message",
        "discord_delete_message",
        "discord_bulk_delete_messages",
    ] {
        let error = policy
            .check_tool(tool)
            .expect_err("a disabled class must refuse its tools");
        assert_eq!(error.category(), ErrorCategory::PolicyBlocked, "{tool}");
    }

    assert_eq!(
        policy
            .check_tool("discord_get_capabilities")
            .expect("reads are enabled by default"),
        ToolRisk::Read
    );
}

#[tokio::test]
async fn the_shared_budget_refuses_a_burst() {
    let fake = Arc::new(FakeDiscord::new());
    let config = PolicyConfig {
        autonomy: AutonomyConfig {
            max_actions_per_minute: 2,
            ..AutonomyConfig::default()
        },
        ..mutation_config()
    };
    let gated = gate(
        Arc::clone(&fake),
        policy(config),
        Arc::new(RecordingAudit::default()),
    );
    let message = MessageId::from_u64(9);

    for _ in 0..2 {
        gated
            .add_reaction(&channel(4), &message, ":wave:", false)
            .await
            .expect("the first two actions are within budget");
    }
    let error = gated
        .add_reaction(&channel(4), &message, ":wave:", false)
        .await
        .expect_err("the third action must be refused");
    assert_eq!(error.category(), ErrorCategory::PolicyBlocked);
    assert!(
        error.message().contains("max_actions_per_minute"),
        "got {error}"
    );
    assert_eq!(
        fake.calls().len(),
        2,
        "only the allowed attempts may reach Discord"
    );
}

#[tokio::test]
async fn a_dry_run_reaches_no_discord_request() {
    let fake = Arc::new(FakeDiscord::new());
    let gated = gate(
        Arc::clone(&fake),
        policy(mutation_config()),
        Arc::new(RecordingAudit::default()),
    );

    let outcome = gated
        .delete_message(&channel(4), &MessageId::from_u64(9), true)
        .await
        .expect("a dry run is a policy decision, not an error");

    assert_eq!(outcome, MutationOutcome::Planned);
    assert!(
        fake.calls().is_empty(),
        "a dry run must not send anything: {:?}",
        fake.calls()
    );
}

#[tokio::test]
async fn a_bulk_delete_of_old_messages_is_refused() {
    let fake = Arc::new(FakeDiscord::new());
    let config = PolicyConfig {
        administrative_enabled: true,
        guild_allowlist: vec!["1".to_string()],
        ..mutation_config()
    };
    let gated = gate(
        Arc::clone(&fake),
        policy(config),
        Arc::new(RecordingAudit::default()),
    );
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("the clock must be after 1970")
        .as_millis() as u64;
    let old = snowflake_at(now_ms - 15 * 24 * 60 * 60 * 1_000);
    let recent = snowflake_at(now_ms - 1_000);

    let error = gated
        .bulk_delete_messages(&channel(4), &[old, recent], false)
        .await
        .expect_err("a 15-day-old message cannot be bulk deleted");
    assert_eq!(error.category(), ErrorCategory::InvalidArgument);
    assert!(
        error.message().contains("older than 14 days"),
        "got {error}"
    );
    assert!(fake.calls().is_empty());
}

#[tokio::test]
async fn a_bulk_delete_above_the_policy_cap_is_refused() {
    let fake = Arc::new(FakeDiscord::new());
    let config = PolicyConfig {
        administrative_enabled: true,
        guild_allowlist: vec!["1".to_string()],
        autonomy: AutonomyConfig {
            max_bulk_delete: 2,
            ..AutonomyConfig::default()
        },
        ..mutation_config()
    };
    let gated = gate(
        Arc::clone(&fake),
        policy(config),
        Arc::new(RecordingAudit::default()),
    );
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("the clock must be after 1970")
        .as_millis() as u64;
    let ids: Vec<MessageId> = (1..=3)
        .map(|offset| snowflake_at(now_ms - offset * 1_000))
        .collect();

    let error = gated
        .bulk_delete_messages(&channel(4), &ids, false)
        .await
        .expect_err("three ids exceed a cap of two");
    assert_eq!(error.category(), ErrorCategory::InvalidArgument);
    assert!(error.message().contains("max_bulk_delete"), "got {error}");
    assert!(fake.calls().is_empty());
}

#[tokio::test]
async fn a_tool_that_requires_dry_run_refuses_a_real_call() {
    let fake = Arc::new(FakeDiscord::new());
    let config = PolicyConfig {
        autonomy: AutonomyConfig {
            require_dry_run: vec!["discord_delete_message".to_string()],
            ..AutonomyConfig::default()
        },
        ..mutation_config()
    };
    let gated = gate(
        Arc::clone(&fake),
        policy(config),
        Arc::new(RecordingAudit::default()),
    );

    let error = gated
        .delete_message(&channel(4), &MessageId::from_u64(9), false)
        .await
        .expect_err("the policy demands a dry run first");
    assert_eq!(error.category(), ErrorCategory::PolicyBlocked);
    assert!(error.message().contains("dry_run"), "got {error}");
    assert!(fake.calls().is_empty());
}

fn new_channel_spec() -> NewChannel {
    NewChannel {
        name: "ops".to_string(),
        kind: ChannelKind::Text,
        parent_id: None,
        topic: None,
        nsfw: false,
    }
}

fn new_thread_spec() -> NewThread {
    NewThread {
        name: "incident".to_string(),
        from_message: None,
        private: false,
        auto_archive_minutes: None,
    }
}

fn new_invite_spec() -> NewInvite {
    NewInvite {
        max_age_seconds: 3_600,
        max_uses: 1,
        temporary: false,
        unique: false,
    }
}

#[tokio::test]
async fn structural_changes_are_refused_until_control_is_enabled() {
    let fake = Arc::new(FakeDiscord::new());
    let gated = gate(
        Arc::clone(&fake),
        policy(PolicyConfig::default()),
        Arc::new(RecordingAudit::default()),
    );

    let errors = [
        gated
            .create_channel(&guild(1), &new_channel_spec(), false)
            .await
            .expect_err("create_channel"),
        gated
            .edit_channel(
                &channel(4),
                &ChannelEdit {
                    name: Some("renamed".to_string()),
                    ..ChannelEdit::default()
                },
                false,
            )
            .await
            .expect_err("edit_channel"),
        gated
            .delete_channel(&channel(4), false)
            .await
            .expect_err("delete_channel"),
        gated
            .set_permission_overwrite(
                &channel(4),
                OverwriteTarget::Role(discord_mcp::discord::ids::RoleId::from_u64(7)),
                PermissionBits::default(),
                PermissionBits::default(),
                false,
            )
            .await
            .expect_err("set_permission_overwrite"),
        gated
            .create_thread(&channel(4), &new_thread_spec(), false)
            .await
            .expect_err("create_thread"),
        gated
            .create_invite(&channel(4), &new_invite_spec(), false)
            .await
            .expect_err("create_invite"),
        gated
            .delete_invite("abc", false)
            .await
            .expect_err("delete_invite"),
    ];

    for error in errors {
        assert_eq!(error.category(), ErrorCategory::PolicyBlocked);
    }
    assert!(
        fake.calls().is_empty(),
        "control is off, so nothing may reach Discord: {:?}",
        fake.calls()
    );
}

#[tokio::test]
async fn structural_dry_runs_plan_without_any_request() {
    let fake = Arc::new(FakeDiscord::new());
    let gated = gate(
        Arc::clone(&fake),
        policy(mutation_config()),
        Arc::new(RecordingAudit::default()),
    );

    let created = gated
        .create_channel(&guild(1), &new_channel_spec(), true)
        .await
        .expect("a dry run is a policy decision");
    let edited = gated
        .edit_channel(
            &channel(4),
            &ChannelEdit {
                topic: Some("new topic".to_string()),
                ..ChannelEdit::default()
            },
            true,
        )
        .await
        .expect("a dry run is a policy decision");
    let thread = gated
        .create_thread(&channel(4), &new_thread_spec(), true)
        .await
        .expect("a dry run is a policy decision");

    assert_eq!(created, MutationOutcome::Planned);
    assert_eq!(edited, MutationOutcome::Planned);
    assert_eq!(thread, MutationOutcome::Planned);
    assert!(fake.calls().is_empty(), "a dry run sends nothing");
}

#[tokio::test]
async fn structural_changes_obey_the_channel_allowlist() {
    let fake = Arc::new(FakeDiscord::new());
    let gated = gate(
        Arc::clone(&fake),
        policy(mutation_config()),
        Arc::new(RecordingAudit::default()),
    );

    let errors = [
        gated
            .delete_channel(&channel(5), false)
            .await
            .expect_err("channel 5 is not allowed"),
        gated
            .create_thread(&channel(5), &new_thread_spec(), false)
            .await
            .expect_err("channel 5 is not allowed"),
        gated
            .create_channel(
                &guild(1),
                &NewChannel {
                    parent_id: Some(channel(5)),
                    ..new_channel_spec()
                },
                false,
            )
            .await
            .expect_err("a parent outside the allowlist is not a foothold"),
    ];

    for error in errors {
        assert_eq!(error.category(), ErrorCategory::PolicyBlocked);
    }
    assert!(fake.calls().is_empty());
}

#[tokio::test]
async fn thread_edits_authorize_through_the_parent_channel() {
    let fake = Arc::new(FakeDiscord::new().with_parent(9, 4));
    let gated = gate(
        Arc::clone(&fake),
        policy(mutation_config()),
        Arc::new(RecordingAudit::default()),
    );

    let outcome = gated
        .edit_thread(&channel(9), Some(true), None, false)
        .await
        .expect("the thread's parent is allowed");

    assert_eq!(outcome, MutationOutcome::Applied(channel_summary(9, None)));
    assert_eq!(
        fake.calls(),
        vec![Call::GetChannel(9), Call::EditThread(9)],
        "scope is resolved once, then the change is sent"
    );
}

#[tokio::test]
async fn a_thread_dry_run_does_not_look_the_thread_up() {
    let fake = Arc::new(FakeDiscord::new().with_parent(9, 4));
    let gated = gate(
        Arc::clone(&fake),
        policy(mutation_config()),
        Arc::new(RecordingAudit::default()),
    );

    let outcome = gated
        .edit_thread(&channel(9), Some(true), None, true)
        .await
        .expect("a dry run reports the local policy decision");

    assert_eq!(outcome, MutationOutcome::Planned);
    assert!(
        fake.calls().is_empty(),
        "a dry run must not resolve remote scope: {:?}",
        fake.calls()
    );
}

#[tokio::test]
async fn a_thread_without_a_guild_is_refused_under_a_guild_allowlist() {
    let fake = Arc::new(FakeDiscord::new().with_parent(9, 4));
    let config = PolicyConfig {
        guild_allowlist: vec!["1".to_string()],
        ..mutation_config()
    };
    let gated = gate(
        Arc::clone(&fake),
        policy(config),
        Arc::new(RecordingAudit::default()),
    );

    let error = gated
        .edit_thread(&channel(9), Some(true), None, false)
        .await
        .expect_err("a guild allowlist cannot verify a guild-less thread");

    assert_eq!(error.category(), ErrorCategory::PolicyBlocked);
    assert!(error.message().contains("no guild"), "got {error}");
    assert_eq!(fake.calls(), vec![Call::GetChannel(9)]);
}

#[tokio::test]
async fn deleting_an_invite_resolves_its_channel_first() {
    let denied_fake = Arc::new(FakeDiscord::new().with_invite("abc", 5));
    let denied = gate(
        Arc::clone(&denied_fake),
        policy(mutation_config()),
        Arc::new(RecordingAudit::default()),
    );
    let error = denied
        .delete_invite("abc", false)
        .await
        .expect_err("the invite belongs to an unlisted channel");
    assert_eq!(error.category(), ErrorCategory::PolicyBlocked);
    assert_eq!(
        denied_fake.calls(),
        vec![Call::GetInvite("abc".to_string())],
        "a refused invite costs no action"
    );

    let allowed_fake = Arc::new(FakeDiscord::new().with_invite("abc", 4));
    let allowed = gate(
        Arc::clone(&allowed_fake),
        policy(mutation_config()),
        Arc::new(RecordingAudit::default()),
    );
    allowed
        .delete_invite("abc", false)
        .await
        .expect("the invite's channel is allowed");
    assert_eq!(
        allowed_fake.calls(),
        vec![
            Call::GetInvite("abc".to_string()),
            Call::DeleteInvite("abc".to_string())
        ]
    );
}

#[tokio::test]
async fn revoking_an_invite_audits_the_resolved_channel() {
    let fake = Arc::new(FakeDiscord::new().with_invite("abc", 4));
    let audit = Arc::new(RecordingAudit::default());
    let gated = gate(
        Arc::clone(&fake),
        policy(mutation_config()),
        Arc::clone(&audit),
    );

    gated
        .delete_invite("abc", false)
        .await
        .expect("the invite's channel is allowed");

    let events = audit.events();
    assert_eq!(
        events.len(),
        1,
        "resolution must not add a record of its own: {events:?}"
    );
    assert_eq!(events[0].action, "discord_delete_invite");
    assert_eq!(events[0].target, AuditTarget::for_channel(channel(4)));
    assert_eq!(events[0].outcome, AuditOutcome::Allowed);
}

#[tokio::test]
async fn the_invite_lookup_is_a_policy_checked_audited_read() {
    let fake = Arc::new(FakeDiscord::new().with_invite("abc", 4));
    let audit = Arc::new(RecordingAudit::default());
    let gated = gate(
        Arc::clone(&fake),
        policy(mutation_config()),
        Arc::clone(&audit),
    );

    let invite = gated.get_invite("abc").await.expect("reads are enabled");

    assert_eq!(invite.channel_id, Some(channel(4)));
    let events = audit.events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].action, "discord_resolve_invite");
    assert_eq!(events[0].outcome, AuditOutcome::Allowed);
}

#[tokio::test]
async fn the_invite_lookup_refuses_when_reads_are_off() {
    let fake = Arc::new(FakeDiscord::new().with_invite("abc", 4));
    let audit = Arc::new(RecordingAudit::default());
    let gated = gate(
        Arc::clone(&fake),
        policy(PolicyConfig {
            read_enabled: false,
            ..mutation_config()
        }),
        Arc::clone(&audit),
    );

    let error = gated
        .get_invite("abc")
        .await
        .expect_err("the read flag must apply to this lookup too");

    assert_eq!(error.category(), ErrorCategory::PolicyBlocked);
    assert!(fake.calls().is_empty(), "the refusal precedes the request");
    assert_eq!(audit.events().len(), 1);
    assert_eq!(audit.events()[0].outcome, AuditOutcome::Denied);
}

#[tokio::test]
async fn an_invite_dry_run_sends_no_request() {
    let fake = Arc::new(FakeDiscord::new().with_invite("abc", 4));
    let gated = gate(
        Arc::clone(&fake),
        policy(mutation_config()),
        Arc::new(RecordingAudit::default()),
    );

    let outcome = gated
        .delete_invite("abc", true)
        .await
        .expect("a dry run reports the local policy decision");

    assert_eq!(outcome, MutationOutcome::Planned);
    assert!(fake.calls().is_empty(), "a dry run must send nothing");
}

#[tokio::test]
async fn structural_changes_share_the_action_budget() {
    let fake = Arc::new(FakeDiscord::new());
    let config = PolicyConfig {
        autonomy: AutonomyConfig {
            max_actions_per_minute: 1,
            ..AutonomyConfig::default()
        },
        ..mutation_config()
    };
    let gated = gate(
        Arc::clone(&fake),
        policy(config),
        Arc::new(RecordingAudit::default()),
    );

    gated
        .create_invite(&channel(4), &new_invite_spec(), false)
        .await
        .expect("the first action is within budget");
    let error = gated
        .create_thread(&channel(4), &new_thread_spec(), false)
        .await
        .expect_err("the budget is shared across control tools");

    assert_eq!(error.category(), ErrorCategory::PolicyBlocked);
    assert!(
        error.message().contains("max_actions_per_minute"),
        "got {error}"
    );
    assert_eq!(fake.calls(), vec![Call::CreateInvite(4)]);
}

fn snowflake_at(millis: u64) -> MessageId {
    const DISCORD_EPOCH_MS: u64 = 1_420_070_400_000;
    MessageId::from_u64((millis - DISCORD_EPOCH_MS) << 22)
}
