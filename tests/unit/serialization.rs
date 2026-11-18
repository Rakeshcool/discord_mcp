//! Tool output shape: identifiers survive JSON as strings, kinds as names.

use discord_mcp::discord::ids::{ChannelId, GuildId, MessageId, UserId};
use discord_mcp::discord::models::{
    ChannelKind, ChannelSummary, GuildSummary, MemberSummary, MessageAuthor, MessageSummary, Page,
};

#[test]
fn snowflakes_serialize_as_decimal_strings() {
    // Larger than f64 can represent exactly: a numeric encoding would corrupt it.
    let id = GuildId::from_u64(1_234_567_890_123_456_789);
    assert_eq!(
        serde_json::to_string(&id).expect("serialize"),
        "\"1234567890123456789\""
    );

    assert_eq!(
        serde_json::to_string(&ChannelId::from_u64(4)).expect("serialize"),
        "\"4\""
    );
    assert_eq!(
        serde_json::to_string(&MessageId::from_u64(9)).expect("serialize"),
        "\"9\""
    );
    assert_eq!(
        serde_json::to_string(&UserId::from_u64(7)).expect("serialize"),
        "\"7\""
    );
}

#[test]
fn channel_kinds_serialize_as_names() {
    assert_eq!(
        serde_json::to_string(&ChannelKind::Voice).expect("serialize"),
        "\"voice\""
    );
    assert_eq!(
        serde_json::to_string(&ChannelKind::Other(99)).expect("serialize"),
        "\"other\""
    );
}

#[test]
fn a_page_of_guilds_keeps_ids_as_strings() {
    let page = Page::new(
        vec![GuildSummary {
            id: GuildId::from_u64(1_234_567_890_123_456_789),
            name: "guild".to_string(),
            icon_hash: None,
            owner: false,
        }],
        Some("1234567890123456789".to_string()),
    );

    let json = serde_json::to_string(&page).expect("serialize");

    assert!(
        json.contains("\"id\":\"1234567890123456789\""),
        "got {json}"
    );
    assert!(
        json.contains("\"next_cursor\":\"1234567890123456789\""),
        "got {json}"
    );
}

#[test]
fn channel_message_and_member_shapes_serialize() {
    let channel = ChannelSummary {
        id: ChannelId::from_u64(4),
        guild_id: Some(GuildId::from_u64(1)),
        name: Some("general".to_string()),
        kind: ChannelKind::Text,
        position: Some(0),
        parent_id: None,
        topic: None,
        nsfw: false,
    };
    let message = MessageSummary {
        id: MessageId::from_u64(9),
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
        embeds: Vec::new(),
        reactions: Vec::new(),
        attachments: Vec::new(),
    };
    let member = MemberSummary {
        user_id: UserId::from_u64(7),
        username: "alice".to_string(),
        nickname: None,
        roles: vec!["8008".to_string()],
        joined_at: None,
    };

    for json in [
        serde_json::to_string(&channel).expect("channel"),
        serde_json::to_string(&message).expect("message"),
        serde_json::to_string(&member).expect("member"),
    ] {
        assert!(json.starts_with('{'), "got {json}");
    }
}
