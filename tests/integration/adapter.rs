//! Adapter behaviour against a mock Discord REST API.
//!
//! Every test drives the real [`TwilightDiscord`] client over a real socket, so
//! what is verified here is the behaviour that ships: URL construction, auth
//! header, pagination bounds, error mapping, retry budget, timeout and the
//! delegation of rate limits to twilight.

use std::sync::Arc;
use std::time::{Duration, Instant};

use discord_mcp::config::schema::DiscordConfig;
use discord_mcp::discord::DiscordApi;
use discord_mcp::discord::adapter::TwilightDiscord;
use discord_mcp::discord::client::{MEMBERS_PAGE_MAX, MutationOutcome};
use discord_mcp::discord::ids::{ChannelId, GuildId, MessageId, RoleId, UserId};
use discord_mcp::discord::models::{
    ChannelEdit, ChannelKind, MessageDraft, MessageQuery, NewChannel, NewEmbed, NewEmbedField,
    NewInvite, NewRole, NewThread, OverwriteTarget, RoleEdit,
};
use discord_mcp::discord::permissions::{PermissionBits, resolve};
use discord_mcp::error::ErrorCategory;
use discord_mcp::security::secrets::BotToken;

use crate::mock_discord::{MockDiscord, MockResponse};

const TOKEN: &str = "MTAwMDAwMDAwMDAwMDAwMDAw.Ga4dEf.abcdefghijklmnopqrstuvwxyz0123456789AB";
const GUILD_ID: &str = "2002";
const CHANNEL_ID: &str = "4004";

fn adapter_for(mock: &MockDiscord) -> TwilightDiscord {
    adapter_with(mock, 2, 2)
}

fn adapter_with(mock: &MockDiscord, timeout_seconds: u64, max_retries: u32) -> TwilightDiscord {
    let config = DiscordConfig {
        api_base: mock.base_url().to_string(),
        allow_insecure_transport: true,
        request_timeout_seconds: timeout_seconds,
        max_retries,
        ..DiscordConfig::default()
    };
    let token = BotToken::from_source("DISCORD_MCP_TEST_TOKEN", Some(TOKEN.to_string()))
        .expect("the test token must satisfy the shape check");
    TwilightDiscord::new(&config, &token).expect("the adapter must build")
}

fn guild(id: &str) -> GuildId {
    GuildId::parse(id).expect("valid guild id")
}

fn channel(id: &str) -> ChannelId {
    ChannelId::parse(id).expect("valid channel id")
}

fn draft(content: &str) -> MessageDraft {
    MessageDraft {
        content: Some(content.to_string()),
        ..MessageDraft::default()
    }
}

fn guild_page() -> &'static str {
    r#"[{"id":"1001","name":"Guild A","icon":null,"owner":false,"permissions":"0","features":[]},
        {"id":"1002","name":"Guild B","icon":"4a1b2c3d4e5f60718293a4b5c6d7e8f9","owner":true,"permissions":"0","features":[]}]"#
}

fn guild_detail() -> &'static str {
    r#"{"id":"2002","name":"Detail","icon":null,"description":"a description",
        "owner_id":"3003","member_count":42,"afk_timeout":300,"large":false,
        "default_message_notifications":0,"explicit_content_filter":0,"mfa_level":0,
        "premium_tier":0,"features":[],"emojis":[],"roles":[],"stickers":[],
        "system_channel_flags":0,"verification_level":0,"nsfw_level":0,
        "preferred_locale":"en-US","premium_progress_bar_enabled":false}"#
}

fn channel_detail() -> &'static str {
    r#"{"id":"4004","type":2,"guild_id":"2002","name":"general","topic":"hello",
        "position":3,"parent_id":"5005","nsfw":false,"permission_overwrites":[]}"#
}

fn role() -> &'static str {
    r#"{"id":"7007","name":"mods","color":0,
        "colors":{"primary_color":16711680,"secondary_color":null,"tertiary_color":null},
        "hoist":true,"position":2,"permissions":"1024","managed":false,
        "mentionable":true,"flags":0}"#
}

fn role_with(id: &str, position: i64) -> String {
    format!(
        r#"{{"id":"{id}","name":"role-{id}","color":0,
            "colors":{{"primary_color":0,"secondary_color":null,"tertiary_color":null}},
            "hoist":false,"position":{position},"permissions":"0","managed":false,
            "mentionable":false,"flags":0}}"#
    )
}

fn bot_member() -> &'static str {
    r#"{"user":{"id":"9999","username":"bot","discriminator":"0","bot":true,
        "avatar":null,"global_name":null,"public_flags":0,"mfa_enabled":false},
        "nick":null,"roles":["7007"],"joined_at":"2024-01-02T03:04:05.000000+00:00",
        "deaf":false,"mute":false,"flags":0,"pending":false}"#
}

fn message(id: &str, content: &str) -> String {
    format!(
        r#"{{"id":"{id}","channel_id":"{CHANNEL_ID}","guild_id":"{GUILD_ID}",
            "content":"{content}","timestamp":"2024-01-01T00:00:00.000000+00:00",
            "edited_timestamp":null,
            "author":{{"id":"7007","username":"alice","discriminator":"0","bot":false,"avatar":null,"global_name":null,"public_flags":0}},
            "attachments":[],"mentions":[],"mention_roles":[],"mention_everyone":false,
            "pinned":false,"tts":false,"type":0,"embeds":[],"components":[],"flags":0}}"#
    )
}

fn rich_message() -> String {
    format!(
        r#"{{"id":"100","channel_id":"{CHANNEL_ID}","guild_id":"{GUILD_ID}",
            "content":"hello","timestamp":"2024-01-01T00:00:00.000000+00:00",
            "edited_timestamp":null,
            "author":{{"id":"7007","username":"alice","discriminator":"0","bot":false,"avatar":null,"global_name":null,"public_flags":0}},
            "attachments":[{{"id":"555","filename":"note.txt","description":"alt text",
                "content_type":"text/plain","size":9,"url":"https://cdn.test/note.txt",
                "proxy_url":"https://media.test/note.txt","height":null,"width":null,
                "ephemeral":false}}],
            "mentions":[],"mention_roles":[],"mention_everyone":false,
            "pinned":false,"tts":false,"type":0,
            "embeds":[{{"type":"rich","title":"Title","description":"Description",
                "url":"https://example.test/title","color":16711680,
                "fields":[{{"name":"Field","value":"Value","inline":true}}],
                "footer":{{"text":"Footer"}},
                "image":{{"url":"https://example.test/image.png"}},
                "thumbnail":{{"url":"https://example.test/thumb.png"}}}}],
            "reactions":[{{"count":3,"me":true,
                "emoji":{{"id":"123","name":"party","animated":true}},
                "burst_colors":[],"count_details":{{"burst":0,"normal":3}},
                "me_burst":false}}],
            "components":[],"flags":0}}"#
    )
}

fn temp_upload(name: &str, contents: &[u8]) -> std::path::PathBuf {
    let directory =
        std::env::temp_dir().join(format!("discord-mcp-adapter-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("create the upload directory");
    let path = directory.join("note.txt");
    std::fs::write(&path, contents).expect("write the upload");
    path
}

fn many_details_message() -> String {
    let embeds = (0..11)
        .map(|index| {
            let title = if index == 0 {
                "x".repeat(300)
            } else {
                format!("Title {index}")
            };
            format!(r#"{{"type":"rich","title":"{title}"}}"#)
        })
        .collect::<Vec<_>>()
        .join(",");
    let reactions = (0..21)
        .map(|index| {
            format!(
                r#"{{"count":1,"me":false,"emoji":{{"name":"e{index}"}},
                    "burst_colors":[],"count_details":{{"burst":0,"normal":1}},
                    "me_burst":false}}"#
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let attachments = (0..11)
        .map(|index| {
            format!(
                r#"{{"id":"{}","filename":"f{index}.txt","size":1,
                    "url":"https://cdn.test/{index}",
                    "proxy_url":"https://media.test/{index}","ephemeral":false}}"#,
                500 + index
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(
        r#"{{"id":"100","channel_id":"{CHANNEL_ID}","guild_id":"{GUILD_ID}",
            "content":"hello","timestamp":"2024-01-01T00:00:00.000000+00:00",
            "edited_timestamp":null,
            "author":{{"id":"7007","username":"alice","discriminator":"0","bot":false,"avatar":null,"global_name":null,"public_flags":0}},
            "attachments":[{attachments}],"mentions":[],"mention_roles":[],
            "mention_everyone":false,"pinned":false,"tts":false,"type":0,
            "embeds":[{embeds}],"reactions":[{reactions}],"components":[],"flags":0}}"#
    )
}

fn member(id: &str, name: &str) -> String {
    format!(
        r#"{{"user":{{"id":"{id}","username":"{name}","discriminator":"0","bot":false,"avatar":null,"global_name":null,"public_flags":0}},
            "nick":null,"roles":["8008"],"joined_at":"2024-01-02T03:04:05.000000+00:00",
            "deaf":false,"mute":false,"flags":0,"pending":false,
            "communication_disabled_until":null,"premium_since":null}}"#
    )
}

#[tokio::test]
async fn list_guilds_maps_the_page_and_sends_the_auth_header() {
    let mock = MockDiscord::start(|_| MockResponse::json(guild_page())).await;
    let api = adapter_for(&mock);

    let page = api.list_guilds(2, None).await.expect("the page must load");

    assert_eq!(page.items.len(), 2);
    assert_eq!(page.items[0].id.as_string(), "1001");
    assert_eq!(page.items[1].name, "Guild B");
    assert!(page.items[1].owner);
    assert_eq!(
        page.items[1].icon_hash.as_deref(),
        Some("4a1b2c3d4e5f60718293a4b5c6d7e8f9")
    );
    assert_eq!(page.next_cursor.as_deref(), Some("1002"));

    let request = &mock.requests()[0];
    assert_eq!(request.method, "GET");
    assert_eq!(request.route(), "/api/v10/users/@me/guilds");
    assert_eq!(request.query("limit").as_deref(), Some("2"));
    assert_eq!(
        request.header("authorization"),
        Some(format!("Bot {TOKEN}").as_str())
    );
}

#[tokio::test]
async fn a_short_page_reports_no_cursor() {
    let mock = MockDiscord::start(|_| {
        MockResponse::json(r#"[{"id":"1001","name":"Only","icon":null,"owner":false,"permissions":"0","features":[]}]"#)
    })
    .await;
    let api = adapter_for(&mock);

    let page = api.list_guilds(2, None).await.expect("the page must load");

    assert_eq!(page.items.len(), 1);
    assert_eq!(page.next_cursor, None);
}

#[tokio::test]
async fn the_after_cursor_is_forwarded() {
    let mock = MockDiscord::start(|_| MockResponse::json("[]")).await;
    let api = adapter_for(&mock);

    api.list_guilds(5, Some(&guild("1002")))
        .await
        .expect("the page must load");

    assert_eq!(
        mock.requests()[0].query("after").as_deref(),
        Some("1002"),
        "the cursor must reach Discord as `after`"
    );
}

#[tokio::test]
async fn get_guild_maps_the_fields_the_tool_will_expose() {
    let mock = MockDiscord::start(|_| MockResponse::json(guild_detail())).await;
    let api = adapter_for(&mock);

    let detail = api.get_guild(&guild(GUILD_ID)).await.expect("guild loads");

    assert_eq!(detail.id.as_string(), GUILD_ID);
    assert_eq!(detail.name, "Detail");
    assert_eq!(detail.description.as_deref(), Some("a description"));
    assert_eq!(
        detail.owner_id.map(|id| id.as_string()).as_deref(),
        Some("3003")
    );
    assert_eq!(detail.member_count, Some(42));
    assert_eq!(mock.requests()[0].route(), "/api/v10/guilds/2002");
}

#[tokio::test]
async fn get_channel_maps_the_channel_type() {
    let mock = MockDiscord::start(|_| MockResponse::json(channel_detail())).await;
    let api = adapter_for(&mock);

    let summary = api
        .get_channel(&channel(CHANNEL_ID))
        .await
        .expect("channel loads");

    assert_eq!(summary.id.as_string(), CHANNEL_ID);
    assert_eq!(summary.name.as_deref(), Some("general"));
    assert_eq!(summary.kind, ChannelKind::Voice);
    assert_eq!(summary.position, Some(3));
    assert_eq!(
        summary.guild_id.map(|id| id.as_string()).as_deref(),
        Some(GUILD_ID)
    );
    assert_eq!(
        summary.parent_id.map(|id| id.as_string()).as_deref(),
        Some("5005")
    );
}

#[tokio::test]
async fn get_messages_bounds_the_page_and_reports_a_cursor() {
    let mock = MockDiscord::start(|_| {
        MockResponse::json(format!(
            "[{},{}]",
            message("9002", "second"),
            message("9001", "")
        ))
    })
    .await;
    let api = adapter_for(&mock);

    let page = api
        .get_messages(&channel(CHANNEL_ID), &MessageQuery::latest(2))
        .await
        .expect("messages load");

    assert_eq!(page.items.len(), 2);
    assert_eq!(page.items[0].content.as_deref(), Some("second"));
    assert_eq!(
        page.items[1].content, None,
        "empty content means content was withheld, not an empty message"
    );
    assert_eq!(page.items[0].author.username, "alice");
    assert_eq!(page.next_cursor.as_deref(), Some("9001"));
    assert_eq!(
        mock.requests()[0].route(),
        "/api/v10/channels/4004/messages"
    );
    assert_eq!(mock.requests()[0].query("limit").as_deref(), Some("2"));
}

#[tokio::test]
async fn get_messages_passes_a_before_cursor() {
    let mock = MockDiscord::start(|_| MockResponse::json("[]")).await;
    let api = adapter_for(&mock);

    let query = MessageQuery {
        limit: 10,
        before: Some(MessageId::parse("9001").expect("valid message id")),
        after: None,
    };
    api.get_messages(&channel(CHANNEL_ID), &query)
        .await
        .expect("messages load");

    assert_eq!(mock.requests()[0].query("before").as_deref(), Some("9001"));
}

#[tokio::test]
async fn mutually_exclusive_cursors_are_rejected_before_any_request() {
    let mock = MockDiscord::start(|_| MockResponse::json("[]")).await;
    let api = adapter_for(&mock);

    let query = MessageQuery {
        limit: 10,
        before: Some(MessageId::parse("9001").expect("valid message id")),
        after: Some(MessageId::parse("9002").expect("valid message id")),
    };
    let error = api
        .get_messages(&channel(CHANNEL_ID), &query)
        .await
        .expect_err("both cursors must be rejected");

    assert_eq!(error.category(), ErrorCategory::InvalidArgument);
    assert_eq!(mock.request_count(), 0, "no request may be sent");
}

#[tokio::test]
async fn a_zero_limit_is_rejected_before_any_request() {
    let mock = MockDiscord::start(|_| MockResponse::json("[]")).await;
    let api = adapter_for(&mock);

    let error = api
        .list_guilds(0, None)
        .await
        .expect_err("a zero limit must be rejected");

    assert_eq!(error.category(), ErrorCategory::InvalidArgument);
    assert_eq!(mock.request_count(), 0);
}

#[tokio::test]
async fn member_pages_are_clamped_to_the_discord_maximum() {
    let mock = MockDiscord::start(|_| MockResponse::json("[]")).await;
    let api = adapter_for(&mock);

    api.list_members(&guild(GUILD_ID), 5_000, None)
        .await
        .expect("members load");

    assert_eq!(
        mock.requests()[0].query("limit").as_deref(),
        Some(MEMBERS_PAGE_MAX.to_string().as_str())
    );
}

#[tokio::test]
async fn list_members_maps_roles_and_usernames() {
    let mock =
        MockDiscord::start(|_| MockResponse::json(format!("[{}]", member("7007", "alice")))).await;
    let api = adapter_for(&mock);

    let page = api
        .list_members(&guild(GUILD_ID), 10, None)
        .await
        .expect("members load");

    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].username, "alice");
    assert_eq!(page.items[0].roles, vec!["8008".to_string()]);
    assert!(page.items[0].joined_at.is_some());
}

#[tokio::test]
async fn a_missing_resource_maps_to_not_found() {
    let mock = MockDiscord::start(|_| MockResponse::error(404, 10004, "Unknown Guild")).await;
    let api = adapter_for(&mock);

    let error = api
        .get_guild(&guild(GUILD_ID))
        .await
        .expect_err("404 must be an error");

    assert_eq!(error.category(), ErrorCategory::ResourceNotFound);
    assert!(!error.is_retryable());
    assert_eq!(mock.request_count(), 1, "a 404 must not be retried");
}

#[tokio::test]
async fn a_permission_failure_maps_to_permission_denied() {
    let mock = MockDiscord::start(|_| MockResponse::error(403, 50001, "Missing Access")).await;
    let api = adapter_for(&mock);

    let error = api
        .list_members(&guild(GUILD_ID), 10, None)
        .await
        .expect_err("403 must be an error");

    assert_eq!(error.category(), ErrorCategory::Permission);
}

#[tokio::test]
async fn a_rejected_token_maps_to_authentication_failed() {
    let mock = MockDiscord::start(|_| MockResponse::error(401, 0, "401: Unauthorized")).await;
    let api = adapter_for(&mock);

    let error = api
        .get_guild(&guild(GUILD_ID))
        .await
        .expect_err("401 must be an error");

    assert_eq!(error.category(), ErrorCategory::Authentication);
}

#[tokio::test]
async fn server_errors_are_retried_within_the_budget() {
    let attempts = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let mock = MockDiscord::start(move |_| {
        let attempt = attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if attempt < 2 {
            MockResponse::error(502, 0, "Bad Gateway")
        } else {
            MockResponse::json(guild_detail())
        }
    })
    .await;
    let api = adapter_with(&mock, 2, 2);

    let detail = api
        .get_guild(&guild(GUILD_ID))
        .await
        .expect("the third attempt must succeed");

    assert_eq!(detail.name, "Detail");
    assert_eq!(mock.request_count(), 3);
}

#[tokio::test]
async fn retries_are_bounded_and_report_the_discord_category() {
    let mock = MockDiscord::start(|_| MockResponse::error(500, 0, "Internal Server Error")).await;
    let api = adapter_with(&mock, 2, 1);

    let error = api
        .get_guild(&guild(GUILD_ID))
        .await
        .expect_err("a persistent 500 must fail");

    assert_eq!(error.category(), ErrorCategory::DiscordApi);
    assert_eq!(mock.request_count(), 2, "one attempt plus one retry");
}

#[tokio::test]
async fn an_unparsable_body_maps_to_network_error() {
    let mock = MockDiscord::start(|_| MockResponse::json("this is not json")).await;
    let api = adapter_with(&mock, 2, 0);

    let error = api
        .get_guild(&guild(GUILD_ID))
        .await
        .expect_err("a malformed body must fail");

    assert_eq!(error.category(), ErrorCategory::Network);
}

#[tokio::test]
async fn a_slow_response_times_out() {
    let mock =
        MockDiscord::start(|_| MockResponse::json(guild_detail()).delay(Duration::from_secs(2)))
            .await;
    let api = adapter_with(&mock, 1, 0);

    let started = Instant::now();
    let error = api
        .get_guild(&guild(GUILD_ID))
        .await
        .expect_err("a slow response must time out");

    assert_eq!(error.category(), ErrorCategory::Timeout);
    assert!(
        started.elapsed() < Duration::from_millis(1_900),
        "the configured timeout must bound the wait"
    );
}

#[tokio::test]
async fn a_rate_limited_response_is_waited_out_and_retried() {
    let attempts = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let mock = MockDiscord::start(move |_| {
        let attempt = attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if attempt == 0 {
            MockResponse::json(
                r#"{"message":"You are being rate limited.","retry_after":1.0,"global":false}"#,
            )
            .status(429)
            .header("Retry-After", "1")
            .header("X-RateLimit-Scope", "shared")
            .header("X-RateLimit-Bucket", "mock-bucket")
        } else {
            MockResponse::json(guild_detail())
        }
    })
    .await;
    let api = adapter_with(&mock, 5, 0);

    let detail = api
        .get_guild(&guild(GUILD_ID))
        .await
        .expect("the request must succeed after waiting out the rate limit");

    assert_eq!(detail.name, "Detail");
    assert!(
        mock.request_count() >= 2,
        "the 429 must be retried after Retry-After"
    );
}

#[tokio::test]
async fn the_trait_is_object_safe_and_usable_through_a_dyn_reference() {
    let mock = MockDiscord::start(|_| MockResponse::json(guild_detail())).await;
    let api: Arc<dyn DiscordApi> = Arc::new(adapter_for(&mock));

    let detail = api.get_guild(&guild(GUILD_ID)).await.expect("guild loads");

    assert_eq!(detail.name, "Detail");
}

#[tokio::test]
async fn an_unsupported_api_version_is_refused_at_construction() {
    let mock = MockDiscord::start(|_| MockResponse::json("[]")).await;
    let config = DiscordConfig {
        api_base: mock.base_url().to_string(),
        api_version: "v9".to_string(),
        allow_insecure_transport: true,
        ..DiscordConfig::default()
    };
    let token = BotToken::from_source("DISCORD_MCP_TEST_TOKEN", Some(TOKEN.to_string()))
        .expect("the test token must satisfy the shape check");

    let error = TwilightDiscord::new(&config, &token)
        .expect_err("a mismatched API version must be refused");

    assert_eq!(error.category(), ErrorCategory::Configuration);
}

#[tokio::test]
async fn the_canonical_base_is_accepted_without_a_proxy() {
    let config = DiscordConfig {
        api_base: "https://discord.com/api".to_string(),
        ..DiscordConfig::default()
    };
    let token = BotToken::from_source("DISCORD_MCP_TEST_TOKEN", Some(TOKEN.to_string()))
        .expect("the test token must satisfy the shape check");

    TwilightDiscord::new(&config, &token).expect("the canonical Discord base must be accepted");
}

#[tokio::test]
async fn a_base_without_the_api_prefix_is_refused() {
    let config = DiscordConfig {
        api_base: "https://discord.com/api/v10".to_string(),
        ..DiscordConfig::default()
    };
    let token = BotToken::from_source("DISCORD_MCP_TEST_TOKEN", Some(TOKEN.to_string()))
        .expect("the test token must satisfy the shape check");

    let error = TwilightDiscord::new(&config, &token)
        .expect_err("the version suffix belongs to the client, not the base URL");

    assert_eq!(error.category(), ErrorCategory::Configuration);
}

#[tokio::test]
async fn a_plaintext_non_loopback_base_is_refused_at_construction() {
    let config = DiscordConfig {
        api_base: "http://discord.example/api".to_string(),
        allow_insecure_transport: true,
        ..DiscordConfig::default()
    };
    let token = BotToken::from_source("DISCORD_MCP_TEST_TOKEN", Some(TOKEN.to_string()))
        .expect("the test token must satisfy the shape check");

    let error = TwilightDiscord::new(&config, &token)
        .expect_err("plaintext to a remote host must be refused");

    assert_eq!(error.category(), ErrorCategory::Configuration);
}

#[tokio::test]
async fn a_sent_message_posts_once_with_mentions_disabled() {
    let mock = MockDiscord::start(|_| MockResponse::json(message("100", "hello"))).await;
    let api = adapter_for(&mock);

    let sent = api
        .send_message(&channel(CHANNEL_ID), &draft("hello"))
        .await
        .expect("the message is created");

    assert_eq!(sent.id, MessageId::from_u64(100));
    assert_eq!(sent.channel_id, channel(CHANNEL_ID));

    let requests = mock.requests();
    assert_eq!(requests.len(), 1, "a write is posted exactly once");
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[0].route(), "/api/v10/channels/4004/messages");
    assert_eq!(requests[0].json()["content"], "hello");
    assert_eq!(
        requests[0].json()["allowed_mentions"]["parse"]
            .as_array()
            .map(Vec::len),
        Some(0),
        "the request itself must disable mentions, not just the text: {:?}",
        requests[0].body
    );
}

#[tokio::test]
async fn a_forbidden_send_maps_to_permission_denied_without_a_retry() {
    let mock = MockDiscord::start(|_| MockResponse::error(403, 50013, "Missing Permissions")).await;
    let api = adapter_for(&mock);

    let error = api
        .send_message(&channel(CHANNEL_ID), &draft("hello"))
        .await
        .expect_err("a 403 must fail");

    assert_eq!(error.category(), ErrorCategory::Permission);
    assert_eq!(
        mock.request_count(),
        1,
        "a refused write is surfaced, not retried"
    );
}

#[tokio::test]
async fn message_reads_include_bounded_embeds_reactions_and_attachments() {
    let mock = MockDiscord::start(|_| MockResponse::json(rich_message())).await;
    let api = adapter_for(&mock);
    let message_id = MessageId::parse("100").expect("valid message id");

    let message = api
        .get_message(&channel(CHANNEL_ID), &message_id)
        .await
        .expect("the rich message must load");

    assert_eq!(message.embeds.len(), 1);
    let embed = &message.embeds[0];
    assert_eq!(embed.kind, "rich");
    assert_eq!(embed.title.as_deref(), Some("Title"));
    assert_eq!(embed.description.as_deref(), Some("Description"));
    assert_eq!(embed.color, Some(16_711_680));
    assert_eq!(embed.fields[0].name, "Field");
    assert_eq!(embed.footer.as_deref(), Some("Footer"));
    assert_eq!(
        embed.image_url.as_deref(),
        Some("https://example.test/image.png")
    );
    assert_eq!(message.reactions.len(), 1);
    assert_eq!(message.reactions[0].emoji, "party");
    assert_eq!(message.reactions[0].emoji_id.as_deref(), Some("123"));
    assert!(message.reactions[0].animated);
    assert_eq!(message.reactions[0].count, 3);
    assert!(message.reactions[0].me);
    assert_eq!(message.attachments.len(), 1);
    assert_eq!(message.attachments[0].id, "555");
    assert_eq!(message.attachments[0].filename, "note.txt");
    assert_eq!(message.attachments[0].size, 9);
    assert_eq!(message.attachment_count, 1);
}

#[tokio::test]
async fn rich_message_details_are_capped_and_long_strings_are_truncated() {
    let mock = MockDiscord::start(|_| MockResponse::json(many_details_message())).await;
    let api = adapter_for(&mock);
    let message_id = MessageId::parse("100").expect("valid message id");

    let message = api
        .get_message(&channel(CHANNEL_ID), &message_id)
        .await
        .expect("the message must load");

    assert_eq!(message.embeds.len(), 10);
    assert_eq!(message.reactions.len(), 20);
    assert_eq!(message.attachments.len(), 10);
    assert_eq!(message.attachment_count, 11);
    let title = message.embeds[0]
        .title
        .as_deref()
        .expect("the first embed has a title");
    assert_eq!(title.chars().count(), 256);
    assert!(title.ends_with('…'));
}

#[tokio::test]
async fn a_rich_send_posts_content_embeds_file_and_reply() {
    let path = temp_upload("rich-send", b"hello file");
    let mock = MockDiscord::start(|_| MockResponse::json(message("100", "hello"))).await;
    let api = adapter_for(&mock);
    let mut message = draft("hello");
    message.embeds.push(NewEmbed {
        title: Some("Title".to_string()),
        description: Some("Description".to_string()),
        fields: vec![NewEmbedField {
            name: "Field".to_string(),
            value: "Value".to_string(),
            inline: true,
        }],
        ..NewEmbed::default()
    });
    message.attachment_paths = vec![path.clone()];
    message.reply_to = Some(MessageId::from_u64(9009));

    api.send_message(&channel(CHANNEL_ID), &message)
        .await
        .expect("the rich send must succeed");

    let requests = mock.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].route(), "/api/v10/channels/4004/messages");
    assert!(
        requests[0]
            .header("content-type")
            .is_some_and(|value| value.starts_with("multipart/form-data")),
        "a file upload must be multipart: {:?}",
        requests[0].header("content-type")
    );
    let body = &requests[0].body;
    assert!(body.contains("\"content\":\"hello\""), "got {body}");
    assert!(body.contains("\"title\":\"Title\""), "got {body}");
    assert!(body.contains("\"message_reference\""), "got {body}");
    assert!(body.contains("filename=\"note.txt\""), "got {body}");
    assert!(body.contains("hello file"), "the file bytes are uploaded");

    std::fs::remove_dir_all(path.parent().expect("upload directory")).ok();
}

#[tokio::test]
async fn a_truncated_response_fails_without_a_retry() {
    let mock =
        MockDiscord::start(|_| MockResponse::json(guild_detail()).declare_length(4096)).await;
    let api = adapter_with(&mock, 2, 3);

    let error = api
        .get_guild(&guild(GUILD_ID))
        .await
        .expect_err("an incomplete body must fail");

    assert_eq!(error.category(), ErrorCategory::Network);
    assert_eq!(
        mock.request_count(),
        1,
        "a body that could not be read is not retried"
    );
}

#[tokio::test]
async fn a_timed_out_write_is_never_resent() {
    let mock = MockDiscord::start(|_| {
        MockResponse::json(message("100", "hello")).delay(Duration::from_secs(2))
    })
    .await;
    let api = adapter_with(&mock, 1, 3);

    let error = api
        .send_message(&channel(CHANNEL_ID), &draft("hello"))
        .await
        .expect_err("the timeout must surface");

    assert_eq!(error.category(), ErrorCategory::Timeout);
    assert_eq!(
        mock.request_count(),
        1,
        "a write whose outcome is unknown must not be resent"
    );
}

#[tokio::test]
async fn a_server_error_on_a_write_is_not_retried() {
    let mock = MockDiscord::start(|_| MockResponse::error(500, 0, "Internal Server Error")).await;
    let api = adapter_with(&mock, 2, 3);

    let error = api
        .send_message(&channel(CHANNEL_ID), &draft("hello"))
        .await
        .expect_err("a persistent 500 must fail");

    assert_eq!(error.category(), ErrorCategory::DiscordApi);
    assert_eq!(
        mock.request_count(),
        1,
        "the read retry budget must not apply to a write"
    );
}

#[tokio::test]
async fn a_rate_limited_send_waits_out_the_limit_and_posts_once() {
    let attempts = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let mock = MockDiscord::start(move |_| {
        if attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
            MockResponse::json(
                r#"{"message":"You are being rate limited.","retry_after":1.0,"global":false}"#,
            )
            .status(429)
            .header("Retry-After", "1")
            .header("X-RateLimit-Scope", "shared")
            .header("X-RateLimit-Bucket", "mock-bucket")
        } else {
            MockResponse::json(message("100", "hello"))
        }
    })
    .await;
    let api = adapter_with(&mock, 5, 0);

    let sent = api
        .send_message(&channel(CHANNEL_ID), &draft("hello"))
        .await
        .expect("a rejected attempt may be resent after the limit clears");

    assert_eq!(sent.id, MessageId::from_u64(100));
    assert_eq!(
        mock.request_count(),
        2,
        "429, then exactly one successful post"
    );
}

#[tokio::test]
async fn edit_message_patches_the_content_and_maps_the_answer() {
    let mock = MockDiscord::start(|request| {
        assert_eq!(request.method, "PATCH");
        assert_eq!(request.route(), "/api/v10/channels/4004/messages/9009");
        assert_eq!(request.json()["content"], "edited");
        MockResponse::json(message("9009", "edited"))
    })
    .await;
    let api = adapter_for(&mock);
    let message_id = MessageId::parse("9009").expect("valid message id");

    let outcome = api
        .edit_message(&channel(CHANNEL_ID), &message_id, "edited", false)
        .await
        .expect("the edit must succeed");

    match outcome {
        MutationOutcome::Applied(edited) => {
            assert_eq!(edited.content.as_deref(), Some("edited"));
        }
        MutationOutcome::Planned => panic!("a real call cannot be planned"),
    }
}

#[tokio::test]
async fn delete_message_sends_one_delete_and_is_never_retried() {
    let mock = MockDiscord::start(|request| {
        assert_eq!(request.method, "DELETE");
        assert_eq!(request.route(), "/api/v10/channels/4004/messages/9009");
        MockResponse::json("").status(204)
    })
    .await;
    let api = adapter_with(&mock, 2, 3);
    let message_id = MessageId::parse("9009").expect("valid message id");

    let outcome = api
        .delete_message(&channel(CHANNEL_ID), &message_id, false)
        .await
        .expect("the delete must succeed");

    assert_eq!(outcome, MutationOutcome::Applied(()));
    assert_eq!(mock.request_count(), 1, "a mutation is never retried");
}

#[tokio::test]
async fn bulk_delete_posts_every_message_id() {
    let mock = MockDiscord::start(|request| {
        assert_eq!(request.method, "POST");
        assert_eq!(
            request.route(),
            "/api/v10/channels/4004/messages/bulk-delete"
        );
        let ids = request.json()["messages"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        assert_eq!(ids.len(), 2);
        MockResponse::json("").status(204)
    })
    .await;
    let api = adapter_for(&mock);
    let ids = [
        MessageId::parse("9001").expect("valid message id"),
        MessageId::parse("9002").expect("valid message id"),
    ];

    let outcome = api
        .bulk_delete_messages(&channel(CHANNEL_ID), &ids, false)
        .await
        .expect("the bulk delete must succeed");

    assert_eq!(outcome, MutationOutcome::Applied(2));
}

#[tokio::test]
async fn add_reaction_targets_the_reaction_route() {
    let mock = MockDiscord::start(|request| {
        assert_eq!(request.method, "PUT");
        assert!(
            request
                .route()
                .starts_with("/api/v10/channels/4004/messages/9009/reactions/"),
            "unexpected route: {}",
            request.route()
        );
        MockResponse::json("").status(204)
    })
    .await;
    let api = adapter_for(&mock);
    let message_id = MessageId::parse("9009").expect("valid message id");

    let outcome = api
        .add_reaction(&channel(CHANNEL_ID), &message_id, "👋", false)
        .await
        .expect("the reaction must be added");

    assert_eq!(outcome, MutationOutcome::Applied(()));
}

#[tokio::test]
async fn a_malformed_custom_emoji_is_an_argument_error() {
    let mock = MockDiscord::start(|_| MockResponse::json("")).await;
    let api = adapter_for(&mock);
    let message_id = MessageId::parse("9009").expect("valid message id");

    let error = api
        .add_reaction(
            &channel(CHANNEL_ID),
            &message_id,
            "wave:not-a-number",
            false,
        )
        .await
        .expect_err("a non-numeric custom id must be refused");

    assert_eq!(error.category(), ErrorCategory::InvalidArgument);
    assert_eq!(mock.request_count(), 0);
}

fn channel_id(value: &str) -> ChannelId {
    ChannelId::parse(value).expect("a valid channel id")
}

#[tokio::test]
async fn create_channel_posts_name_type_parent_and_nsfw() {
    let mock = MockDiscord::start(|request| {
        assert_eq!(request.method, "POST");
        assert_eq!(request.route(), "/api/v10/guilds/2002/channels");
        let body = request.json();
        assert_eq!(body["name"], "ops");
        assert_eq!(body["type"], 0);
        assert_eq!(body["parent_id"], "5005");
        assert_eq!(body["nsfw"], true);
        MockResponse::json(channel_detail())
    })
    .await;
    let api = adapter_for(&mock);
    let spec = NewChannel {
        name: "ops".to_string(),
        kind: ChannelKind::Text,
        parent_id: Some(channel_id("5005")),
        topic: None,
        nsfw: true,
    };

    let outcome = api
        .create_channel(
            &GuildId::parse(GUILD_ID).expect("valid guild id"),
            &spec,
            false,
        )
        .await
        .expect("the channel must be created");

    match outcome {
        MutationOutcome::Applied(created) => assert_eq!(created.id, channel_id(CHANNEL_ID)),
        MutationOutcome::Planned => panic!("a real call cannot be planned"),
    }
}

#[tokio::test]
async fn edit_channel_patches_only_the_fields_given() {
    let mock = MockDiscord::start(|request| {
        assert_eq!(request.method, "PATCH");
        assert_eq!(request.route(), "/api/v10/channels/4004");
        let body = request.json();
        assert_eq!(body["name"], "renamed");
        assert_eq!(body["rate_limit_per_user"], 30);
        assert!(body["topic"].is_null(), "an unset field must stay unset");
        MockResponse::json(channel_detail())
    })
    .await;
    let api = adapter_for(&mock);
    let spec = ChannelEdit {
        name: Some("renamed".to_string()),
        slowmode_seconds: Some(30),
        ..ChannelEdit::default()
    };

    api.edit_channel(&channel(CHANNEL_ID), &spec, false)
        .await
        .expect("the channel must be edited");
}

#[tokio::test]
async fn delete_channel_sends_one_delete() {
    let mock = MockDiscord::start(|request| {
        assert_eq!(request.method, "DELETE");
        assert_eq!(request.route(), "/api/v10/channels/4004");
        // Discord answers a channel delete with the deleted channel.
        MockResponse::json(channel_detail())
    })
    .await;
    let api = adapter_for(&mock);

    let outcome = api
        .delete_channel(&channel(CHANNEL_ID), false)
        .await
        .expect("the channel must be deleted");

    assert_eq!(outcome, MutationOutcome::Applied(()));
    assert_eq!(mock.request_count(), 1);
}

#[tokio::test]
async fn a_permission_overwrite_is_put_for_the_target() {
    let allow = resolve(&["VIEW_CHANNEL".to_string()], "allow", false).expect("known name");
    let mock = MockDiscord::start(|request| {
        assert_eq!(request.method, "PUT");
        assert_eq!(request.route(), "/api/v10/channels/4004/permissions/7007");
        let body = request.json();
        assert_eq!(body["type"], 0, "0 is a role overwrite");
        assert!(
            body["allow"].to_string().contains("1024"),
            "VIEW_CHANNEL is bit 10: {}",
            body["allow"]
        );
        MockResponse::json("").status(204)
    })
    .await;
    let api = adapter_for(&mock);

    api.set_permission_overwrite(
        &channel(CHANNEL_ID),
        OverwriteTarget::Role(RoleId::parse("7007").expect("valid role id")),
        allow,
        PermissionBits::default(),
        false,
    )
    .await
    .expect("the overwrite must be set");
}

#[tokio::test]
async fn a_permission_overwrite_is_deleted_for_the_target() {
    let mock = MockDiscord::start(|request| {
        assert_eq!(request.method, "DELETE");
        assert_eq!(request.route(), "/api/v10/channels/4004/permissions/7007");
        MockResponse::json("").status(204)
    })
    .await;
    let api = adapter_for(&mock);

    api.delete_permission_overwrite(
        &channel(CHANNEL_ID),
        OverwriteTarget::Role(RoleId::parse("7007").expect("valid role id")),
        false,
    )
    .await
    .expect("the overwrite must be deleted");
}

#[tokio::test]
async fn create_thread_posts_a_public_thread_with_its_archive_delay() {
    let mock = MockDiscord::start(|request| {
        assert_eq!(request.method, "POST");
        assert_eq!(request.route(), "/api/v10/channels/4004/threads");
        let body = request.json();
        assert_eq!(body["name"], "incident");
        assert_eq!(body["type"], 11, "11 is a public thread");
        assert_eq!(body["auto_archive_duration"], 1440);
        MockResponse::json(channel_detail())
    })
    .await;
    let api = adapter_for(&mock);
    let spec = NewThread {
        name: "incident".to_string(),
        from_message: None,
        private: false,
        auto_archive_minutes: Some(1_440),
    };

    api.create_thread(&channel(CHANNEL_ID), &spec, false)
        .await
        .expect("the thread must be created");
}

#[tokio::test]
async fn edit_thread_archives_it() {
    let mock = MockDiscord::start(|request| {
        assert_eq!(request.method, "PATCH");
        assert_eq!(request.route(), "/api/v10/channels/4004");
        assert_eq!(request.json()["archived"], true);
        MockResponse::json(channel_detail())
    })
    .await;
    let api = adapter_for(&mock);

    api.edit_thread(&channel(CHANNEL_ID), Some(true), None, false)
        .await
        .expect("the thread must be archived");
}

#[tokio::test]
async fn create_invite_sends_its_limits_and_maps_the_code() {
    let mock = MockDiscord::start(|request| {
        assert_eq!(request.method, "POST");
        assert_eq!(request.route(), "/api/v10/channels/4004/invites");
        let body = request.json();
        assert_eq!(body["max_age"], 3600);
        assert_eq!(body["max_uses"], 1);
        assert_eq!(body["temporary"], true);
        assert_eq!(body["unique"], true);
        MockResponse::json(
            r#"{"code":"abc123","type":0,"channel":{"id":"4004","name":"general","type":0}}"#,
        )
    })
    .await;
    let api = adapter_for(&mock);
    let spec = NewInvite {
        max_age_seconds: 3_600,
        max_uses: 1,
        temporary: true,
        unique: true,
    };

    let outcome = api
        .create_invite(&channel(CHANNEL_ID), &spec, false)
        .await
        .expect("the invite must be created");

    match outcome {
        MutationOutcome::Applied(invite) => {
            assert_eq!(invite.code, "abc123");
            assert_eq!(invite.channel_id, Some(channel_id(CHANNEL_ID)));
        }
        MutationOutcome::Planned => panic!("a real call cannot be planned"),
    }
}

#[tokio::test]
async fn create_role_posts_its_flags_permissions_and_color() {
    let mock = MockDiscord::start(|request| {
        assert_eq!(request.method, "POST");
        assert_eq!(request.route(), "/api/v10/guilds/2002/roles");
        let body = request.json();
        assert_eq!(body["name"], "mods");
        assert!(body["permissions"].to_string().contains("1024"));
        assert_eq!(body["color"], 16711680);
        assert_eq!(body["hoist"], true);
        assert_eq!(body["mentionable"], true);
        MockResponse::json(role())
    })
    .await;
    let api = adapter_for(&mock);
    let spec = NewRole {
        name: "mods".to_string(),
        permissions: resolve(&["VIEW_CHANNEL".to_string()], "permissions", false)
            .expect("known name"),
        color: Some(16_711_680),
        hoist: true,
        mentionable: true,
    };

    let outcome = api
        .create_role(
            &GuildId::parse(GUILD_ID).expect("valid guild id"),
            &spec,
            false,
        )
        .await
        .expect("the role must be created");

    match outcome {
        MutationOutcome::Applied(created) => {
            assert_eq!(created.id, RoleId::parse("7007").expect("valid role id"));
            assert_eq!(created.color, 16_711_680);
            assert_eq!(created.permissions, 1 << 10);
            assert!(created.hoist && created.mentionable);
        }
        MutationOutcome::Planned => panic!("a real call cannot be planned"),
    }
}

#[tokio::test]
async fn edit_role_patches_only_the_fields_given() {
    let mock = MockDiscord::start(|request| {
        assert_eq!(request.method, "PATCH");
        assert_eq!(request.route(), "/api/v10/guilds/2002/roles/7007");
        let body = request.json();
        assert_eq!(body["name"], "renamed");
        assert_eq!(body["hoist"], true);
        assert!(
            body["permissions"].is_null(),
            "an unset field must stay unset"
        );
        MockResponse::json(role())
    })
    .await;
    let api = adapter_for(&mock);
    let spec = RoleEdit {
        name: Some("renamed".to_string()),
        hoist: Some(true),
        ..RoleEdit::default()
    };

    api.edit_role(
        &GuildId::parse(GUILD_ID).expect("valid guild id"),
        &RoleId::parse("7007").expect("valid role id"),
        &spec,
        false,
    )
    .await
    .expect("the role must be edited");
}

#[tokio::test]
async fn highest_role_of_picks_the_top_role_the_member_holds() {
    let mock = MockDiscord::start(|request| match request.route() {
        "/api/v10/guilds/2002/members/9999" => MockResponse::json(bot_member()),
        "/api/v10/guilds/2002/roles" => MockResponse::json(format!(
            "[{},{},{}]",
            role_with("2002", 0),
            role_with("7007", 2),
            role_with("8008", 5)
        )),
        other => panic!("unexpected route: {other}"),
    })
    .await;
    let api = adapter_for(&mock);

    let highest = api
        .highest_role_of(
            &GuildId::parse(GUILD_ID).expect("valid guild id"),
            &UserId::parse("9999").expect("valid user id"),
        )
        .await
        .expect("the member's roles must load");

    assert_eq!(
        highest,
        Some(RoleId::parse("7007").expect("valid role id")),
        "the highest role the bot actually holds, not the guild's highest role"
    );
}

#[tokio::test]
async fn delete_role_sends_one_delete() {
    let mock = MockDiscord::start(|request| {
        assert_eq!(request.method, "DELETE");
        assert_eq!(request.route(), "/api/v10/guilds/2002/roles/7007");
        MockResponse::json("").status(204)
    })
    .await;
    let api = adapter_for(&mock);

    let outcome = api
        .delete_role(
            &GuildId::parse(GUILD_ID).expect("valid guild id"),
            &RoleId::parse("7007").expect("valid role id"),
            false,
        )
        .await
        .expect("the role must be deleted");

    assert_eq!(outcome, MutationOutcome::Applied(()));
    assert_eq!(mock.request_count(), 1);
}

#[tokio::test]
async fn assign_role_puts_the_member_role_route() {
    let mock = MockDiscord::start(|request| {
        assert_eq!(request.method, "PUT");
        assert_eq!(
            request.route(),
            "/api/v10/guilds/2002/members/9009/roles/7007"
        );
        MockResponse::json("").status(204)
    })
    .await;
    let api = adapter_for(&mock);

    api.assign_role(
        &GuildId::parse(GUILD_ID).expect("valid guild id"),
        &UserId::parse("9009").expect("valid user id"),
        &RoleId::parse("7007").expect("valid role id"),
        false,
    )
    .await
    .expect("the role must be assigned");
}

#[tokio::test]
async fn remove_role_deletes_the_member_role_route() {
    let mock = MockDiscord::start(|request| {
        assert_eq!(request.method, "DELETE");
        assert_eq!(
            request.route(),
            "/api/v10/guilds/2002/members/9009/roles/7007"
        );
        MockResponse::json("").status(204)
    })
    .await;
    let api = adapter_for(&mock);

    api.remove_role(
        &GuildId::parse(GUILD_ID).expect("valid guild id"),
        &UserId::parse("9009").expect("valid user id"),
        &RoleId::parse("7007").expect("valid role id"),
        false,
    )
    .await
    .expect("the role must be removed");
}

#[tokio::test]
async fn nickname_changes_patch_the_member_and_clear_sends_null() {
    let mock = MockDiscord::start(|request| {
        assert_eq!(request.method, "PATCH");
        assert_eq!(request.route(), "/api/v10/guilds/2002/members/9009");
        assert_eq!(request.header("x-audit-log-reason"), Some("nickname"));
        MockResponse::json(member("9009", "alice"))
    })
    .await;
    let api = adapter_for(&mock);

    api.set_member_nickname(
        &GuildId::parse(GUILD_ID).expect("valid guild id"),
        &UserId::parse("9009").expect("valid user id"),
        Some("mod"),
        Some("nickname"),
        false,
    )
    .await
    .expect("the nickname must be set");
    api.set_member_nickname(
        &GuildId::parse(GUILD_ID).expect("valid guild id"),
        &UserId::parse("9009").expect("valid user id"),
        None,
        Some("nickname"),
        false,
    )
    .await
    .expect("the nickname must be cleared");

    let requests = mock.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].json()["nick"], "mod");
    assert!(
        requests[1].json()["nick"].is_null(),
        "an empty nickname must serialize as null"
    );
}

#[tokio::test]
async fn kick_and_unban_use_the_member_and_ban_routes() {
    let mock = MockDiscord::start(|request| match request.route() {
        "/api/v10/guilds/2002/members/9009" => {
            assert_eq!(request.method, "DELETE");
            assert_eq!(request.header("x-audit-log-reason"), Some("kick"));
            MockResponse::json("").status(204)
        }
        "/api/v10/guilds/2002/bans/9009" => {
            assert_eq!(request.method, "DELETE");
            assert_eq!(request.header("x-audit-log-reason"), Some("unban"));
            MockResponse::json("").status(204)
        }
        other => panic!("unexpected route: {other}"),
    })
    .await;
    let api = adapter_for(&mock);

    api.kick_member(
        &GuildId::parse(GUILD_ID).expect("valid guild id"),
        &UserId::parse("9009").expect("valid user id"),
        Some("kick"),
        false,
    )
    .await
    .expect("the member must be kicked");
    api.unban_member(
        &GuildId::parse(GUILD_ID).expect("valid guild id"),
        &UserId::parse("9009").expect("valid user id"),
        Some("unban"),
        false,
    )
    .await
    .expect("the ban must be removed");
}

#[tokio::test]
async fn ban_posts_the_message_deletion_window() {
    let mock = MockDiscord::start(|request| {
        assert_eq!(request.method, "PUT");
        assert_eq!(request.route(), "/api/v10/guilds/2002/bans/9009");
        assert_eq!(request.header("x-audit-log-reason"), Some("ban"));
        let body = request.json();
        assert_eq!(body["delete_message_seconds"], 86_400);
        MockResponse::json("").status(204)
    })
    .await;
    let api = adapter_for(&mock);

    api.ban_member(
        &GuildId::parse(GUILD_ID).expect("valid guild id"),
        &UserId::parse("9009").expect("valid user id"),
        86_400,
        Some("ban"),
        false,
    )
    .await
    .expect("the member must be banned");
}

#[tokio::test]
async fn timeout_patches_a_future_expiry_and_clear_sends_null() {
    let mock = MockDiscord::start(|request| {
        assert_eq!(request.method, "PATCH");
        assert_eq!(request.route(), "/api/v10/guilds/2002/members/9009");
        assert_eq!(request.header("x-audit-log-reason"), Some("timeout"));
        MockResponse::json(member("9009", "alice"))
    })
    .await;
    let api = adapter_for(&mock);
    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after 1970")
        .as_secs();

    api.timeout_member(
        &GuildId::parse(GUILD_ID).expect("valid guild id"),
        &UserId::parse("9009").expect("valid user id"),
        3_600,
        Some("timeout"),
        false,
    )
    .await
    .expect("the member must be timed out");
    api.clear_member_timeout(
        &GuildId::parse(GUILD_ID).expect("valid guild id"),
        &UserId::parse("9009").expect("valid user id"),
        Some("timeout"),
        false,
    )
    .await
    .expect("the timeout must be cleared");

    let requests = mock.requests();
    let until = requests[0].json()["communication_disabled_until"]
        .as_str()
        .expect("a timeout timestamp")
        .to_string();
    let timestamp = twilight_model::util::Timestamp::parse(&until).expect("an ISO timestamp");
    let seconds = timestamp.as_secs() as u64;
    assert!(
        seconds >= before + 3_500 && seconds <= before + 3_700,
        "the timeout must be about an hour in the future, got {seconds} from {before}"
    );
    assert!(
        requests[1].json()["communication_disabled_until"].is_null(),
        "clearing a timeout must serialize as null"
    );
}

#[tokio::test]
async fn delete_invite_sends_one_delete_by_code() {
    let mock = MockDiscord::start(|request| {
        assert_eq!(request.method, "DELETE");
        assert_eq!(request.route(), "/api/v10/invites/abc123");
        MockResponse::json("").status(204)
    })
    .await;
    let api = adapter_for(&mock);

    let outcome = api
        .delete_invite("abc123", false)
        .await
        .expect("the invite must be revoked");

    assert_eq!(outcome, MutationOutcome::Applied(()));
}
