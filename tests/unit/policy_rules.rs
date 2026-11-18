//! Policy rules: classification, allowlists, limits and mention neutralisation.

use discord_mcp::config::Config;
use discord_mcp::config::schema::{AutonomyConfig, PolicyConfig};
use discord_mcp::discord::ids::{ChannelId, GuildId};
use discord_mcp::error::ErrorCategory;
use discord_mcp::policy::rules::{Policy, ToolRisk, classify_tool, sanitize_mentions};

const READ_TOOLS: [&str; 8] = [
    "discord_list_guilds",
    "discord_get_guild",
    "discord_list_channels",
    "discord_get_channel",
    "discord_get_messages",
    "discord_get_message",
    "discord_list_members",
    "discord_get_member",
];

fn policy(config: PolicyConfig) -> Policy {
    Policy::from_config(&config).expect("the test configuration must produce a policy")
}

fn guild(id: u64) -> GuildId {
    GuildId::from_u64(id)
}

fn channel(id: u64) -> ChannelId {
    ChannelId::from_u64(id)
}

#[test]
fn every_read_tool_is_classified_as_a_read() {
    for name in READ_TOOLS {
        assert_eq!(classify_tool(name), Some(ToolRisk::Read), "{name}");
    }
    assert_eq!(ToolRisk::Read.as_str(), "read");
}

#[test]
fn the_write_tool_is_classified_as_a_write() {
    assert_eq!(classify_tool("discord_send_message"), Some(ToolRisk::Write));
    assert_eq!(ToolRisk::Write.as_str(), "write");
}

#[test]
fn unclassified_tools_are_denied_rather_than_defaulted() {
    let policy = policy(PolicyConfig::default());

    assert_eq!(classify_tool("discord_grid_clear"), None);
    assert_eq!(classify_tool(""), None);
    let error = policy
        .check_tool("discord_grid_clear")
        .expect_err("an unknown tool must not be allowed");
    assert_eq!(error.category(), ErrorCategory::PolicyBlocked);
}

#[test]
fn destructive_and_administrative_tools_have_their_own_classes() {
    assert_eq!(
        classify_tool("discord_delete_message"),
        Some(ToolRisk::Destructive)
    );
    assert_eq!(
        classify_tool("discord_bulk_delete_messages"),
        Some(ToolRisk::Administrative)
    );
    assert_eq!(ToolRisk::Destructive.as_str(), "destructive");
    assert_eq!(ToolRisk::Administrative.as_str(), "administrative");
}

#[test]
fn a_dry_run_entry_must_name_a_classified_tool() {
    let error = Policy::from_config(&PolicyConfig {
        autonomy: AutonomyConfig {
            require_dry_run: vec!["discord_grid_clear".to_string()],
            ..AutonomyConfig::default()
        },
        ..PolicyConfig::default()
    })
    .expect_err("an unknown tool name would make the setting silently inert");

    assert_eq!(error.category(), ErrorCategory::Configuration);
    assert!(error.message().contains("require_dry_run"), "got {error}");
}

#[test]
fn reads_require_read_enabled() {
    let disabled = policy(PolicyConfig {
        read_enabled: false,
        ..PolicyConfig::default()
    });
    let error = disabled
        .check_tool("discord_get_guild")
        .expect_err("reads must be disabled");
    assert_eq!(error.category(), ErrorCategory::PolicyBlocked);

    let enabled = policy(PolicyConfig::default());
    assert_eq!(enabled.check_tool("discord_get_guild"), Ok(ToolRisk::Read));
}

#[test]
fn writes_require_write_enabled() {
    let defaults = policy(PolicyConfig::default());
    let error = defaults
        .check_tool("discord_send_message")
        .expect_err("writes are off by default");
    assert_eq!(error.category(), ErrorCategory::PolicyBlocked);

    let enabled = policy(PolicyConfig {
        write_enabled: true,
        channel_allowlist: vec!["4".to_string()],
        ..PolicyConfig::default()
    });
    assert_eq!(
        enabled.check_tool("discord_send_message"),
        Ok(ToolRisk::Write)
    );
}

#[test]
fn a_zero_or_oversized_page_is_rejected() {
    let policy = policy(PolicyConfig {
        max_messages_per_read: 10,
        ..PolicyConfig::default()
    });

    assert_eq!(
        policy.check_page_limit(0).expect_err("zero").category(),
        ErrorCategory::InvalidArgument
    );
    assert_eq!(
        policy
            .check_page_limit(11)
            .expect_err("above the policy ceiling")
            .category(),
        ErrorCategory::InvalidArgument
    );
    assert!(policy.check_page_limit(1).is_ok());
    assert!(policy.check_page_limit(10).is_ok());
}

#[test]
fn message_length_is_measured_in_characters() {
    let policy = policy(PolicyConfig {
        write_enabled: true,
        channel_allowlist: vec!["4".to_string()],
        max_message_characters: 10,
        ..PolicyConfig::default()
    });

    assert_eq!(
        policy
            .prepare_outgoing_message("   ")
            .expect_err("empty content")
            .category(),
        ErrorCategory::InvalidArgument
    );
    assert_eq!(
        policy
            .prepare_outgoing_message("abcdefghijk")
            .expect_err("too long")
            .category(),
        ErrorCategory::InvalidArgument
    );
    // Ten multi-byte characters are ten characters, not twenty bytes.
    assert!(policy.prepare_outgoing_message("éééééééééé").is_ok());
}

#[test]
fn outgoing_messages_require_writes_to_be_enabled() {
    let policy = policy(PolicyConfig::default());
    let error = policy
        .prepare_outgoing_message("hello")
        .expect_err("writes are off");
    assert_eq!(error.category(), ErrorCategory::PolicyBlocked);
}

#[test]
fn plain_text_passes_through_untouched() {
    let sanitized = sanitize_mentions("hello there, no mentions", false);

    assert_eq!(sanitized.content, "hello there, no mentions");
    assert_eq!(sanitized.suppressed_mentions, 0);
    assert!(!sanitized.changed());
}

#[test]
fn mass_mentions_are_neutralised() {
    let sanitized = sanitize_mentions("@everyone look at this", false);

    assert_eq!(sanitized.content, "@\u{200b}everyone look at this");
    assert_eq!(sanitized.suppressed_mentions, 1);
    assert!(sanitized.changed());
    assert!(!sanitized.content.contains("@everyone"));
}

#[test]
fn here_mentions_are_neutralised() {
    let sanitized = sanitize_mentions("ping @here", false);

    assert_eq!(sanitized.content, "ping @\u{200b}here");
    assert_eq!(sanitized.suppressed_mentions, 1);
}

#[test]
fn user_and_role_mentions_are_neutralised() {
    for (input, expected) in [
        ("<@123>", "<\u{200b}@123>"),
        ("<@!123>", "<\u{200b}@!123>"),
        ("<@&456>", "<\u{200b}@&456>"),
    ] {
        let sanitized = sanitize_mentions(input, false);
        assert_eq!(sanitized.content, expected, "{input}");
        assert_eq!(sanitized.suppressed_mentions, 1, "{input}");
    }
}

#[test]
fn every_mention_in_a_message_is_counted() {
    let sanitized = sanitize_mentions("@everyone <@1> hi <@&2> @here", false);

    assert_eq!(sanitized.suppressed_mentions, 4);
    for marker in ["@everyone", "@here", "<@1>", "<@&2>"] {
        assert!(
            !sanitized.content.contains(marker),
            "`{marker}` survived in {:?}",
            sanitized.content
        );
    }
}

#[test]
fn neutralising_twice_changes_nothing_further() {
    let once = sanitize_mentions("@everyone <@1>", false);
    let twice = sanitize_mentions(&once.content, false);

    assert_eq!(twice.content, once.content);
    assert_eq!(twice.suppressed_mentions, 0);
}

#[test]
fn mentions_are_kept_only_when_configuration_allows_them() {
    let sanitized = sanitize_mentions("@everyone", true);

    assert_eq!(sanitized.content, "@everyone");
    assert_eq!(sanitized.suppressed_mentions, 0);
}

#[test]
fn outgoing_messages_are_sanitised_before_they_leave() {
    let policy = policy(PolicyConfig {
        write_enabled: true,
        channel_allowlist: vec!["4".to_string()],
        ..PolicyConfig::default()
    });

    let prepared = policy
        .prepare_outgoing_message("@everyone hello")
        .expect("a short message");

    assert_eq!(prepared.content, "@\u{200b}everyone hello");
    assert_eq!(prepared.suppressed_mentions, 1);
}

#[test]
fn allowlists_are_sorted_and_de_duplicated() {
    let policy = policy(PolicyConfig {
        guild_allowlist: vec!["3".to_string(), "1".to_string(), "3".to_string()],
        ..PolicyConfig::default()
    });

    assert!(policy.check_guild(&guild(1)).is_ok());
    assert!(policy.check_guild(&guild(3)).is_ok());
    assert!(policy.check_guild(&guild(2)).is_err());
    assert!(policy.guild_restricted());
}

#[test]
fn an_unparsable_allowlist_entry_is_a_configuration_failure() {
    let error = Policy::from_config(&PolicyConfig {
        guild_allowlist: vec!["not-a-snowflake".to_string()],
        ..PolicyConfig::default()
    })
    .expect_err("an invalid allowlist must not be treated as empty");

    assert_eq!(error.category(), ErrorCategory::Configuration);
}

#[test]
fn the_shipped_defaults_allow_reads_without_an_allowlist() {
    let policy = policy(Config::default().policy);

    assert!(!policy.guild_restricted());
    assert!(policy.check_guild(&guild(1)).is_ok());
    assert!(policy.check_channel(&channel(4)).is_ok());
    assert!(policy.check_read_enabled().is_ok());
    assert!(policy.check_write_enabled().is_err());
}
