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
use discord_mcp::discord::client::MEMBERS_PAGE_MAX;
use discord_mcp::discord::ids::{ChannelId, GuildId, MessageId};
use discord_mcp::discord::models::{ChannelKind, MessageQuery};
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
