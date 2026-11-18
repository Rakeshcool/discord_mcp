//! Configuration defaults and fail-closed validation.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use discord_mcp::config::TransportMode;
use discord_mcp::config::load::{load, load_file};

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn temp_config(text: &str) -> PathBuf {
    let index = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "discord-mcp-config-{}-{index}.toml",
        std::process::id()
    ));
    std::fs::write(&path, text).expect("write the temporary configuration file");
    path
}

/// Load `text` and return the rejection message; panic if it is accepted.
fn rejection(text: &str) -> String {
    let path = temp_config(text);
    let result = load_file(&path);
    std::fs::remove_file(&path).ok();
    match result {
        Ok(_) => panic!("configuration should have been rejected:\n{text}"),
        Err(error) => error.to_string(),
    }
}

/// Load `text` and require it to be accepted.
fn accepted(text: &str) {
    let path = temp_config(text);
    let result = load_file(&path);
    std::fs::remove_file(&path).ok();
    if let Err(error) = result {
        panic!("configuration should have been accepted but was rejected: {error}\n{text}");
    }
}

#[test]
fn embedded_defaults_are_valid_and_safe() {
    let config = load(None).expect("the built-in defaults must always load");

    assert_eq!(config.server.name, "discord-mcp");
    assert_eq!(config.server.transport, TransportMode::Stdio);
    assert_eq!(config.http.host, "127.0.0.1");
    assert!(!config.http.allow_non_loopback);
    assert!(config.http.require_auth);
    assert_eq!(config.discord.api_version, "v10");
    assert!(config.discord.api_base.starts_with("https://"));
    assert!(!config.policy.write_enabled);
    assert!(!config.policy.allow_mass_mentions);
    assert_eq!(config.policy.max_messages_per_read, 100);
    assert_eq!(config.policy.max_message_characters, 2_000);
    assert_eq!(config.policy.max_embeds_per_message, 10);
    assert_eq!(config.policy.max_embed_characters, 6_000);
    assert_eq!(config.policy.max_attachments_per_message, 10);
    assert_eq!(config.policy.max_attachment_bytes, 8_388_608);
    assert_eq!(config.policy.max_attachment_total_bytes, 16_777_216);
    assert!(config.policy.attachment_root.is_none());
    assert!(config.logging.redact_message_content);
    assert!(!config.logging.log_tool_arguments);
}

#[test]
fn the_removed_confirmation_setting_is_rejected() {
    // The setting defaulted to true but no tool path consulted it. Hiding it from
    // the schema would silently ignore an operator's intent, so a configuration
    // that still contains it must fail instead of loading.
    let message = rejection("[policy]\nrequire_confirmation_for_destructive = false\n");
    assert!(
        message.contains("require_confirmation_for_destructive"),
        "unexpected message: {message}"
    );
}

#[test]
fn write_tools_require_a_channel_allowlist() {
    let message = rejection("[policy]\nwrite_enabled = true\n");
    assert!(
        message.contains("policy.channel_allowlist"),
        "unexpected message: {message}"
    );
}

#[test]
fn write_tools_are_allowed_with_an_allowlist() {
    accepted("[policy]\nwrite_enabled = true\nchannel_allowlist = [\"123456789012345678\"]\n");
}

#[test]
fn mass_mentions_are_always_rejected() {
    let message = rejection("[policy]\nallow_mass_mentions = true\n");
    assert!(
        message.contains("policy.allow_mass_mentions"),
        "unexpected message: {message}"
    );
}

#[test]
fn non_loopback_binding_requires_opt_in() {
    let message = rejection("[http]\nhost = \"0.0.0.0\"\n");
    assert!(
        message.contains("http.host"),
        "unexpected message: {message}"
    );
    assert!(
        message.contains("allow_non_loopback"),
        "unexpected message: {message}"
    );
}

#[test]
fn non_loopback_binding_requires_authentication() {
    let message =
        rejection("[http]\nhost = \"0.0.0.0\"\nallow_non_loopback = true\nrequire_auth = false\n");
    assert!(
        message.contains("http.require_auth"),
        "unexpected message: {message}"
    );
}

#[test]
fn the_in_flight_cap_is_bounded() {
    for value in ["0", "1025"] {
        let message = rejection(&format!("[http]\nmax_in_flight_requests = {value}\n"));
        assert!(
            message.contains("http.max_in_flight_requests"),
            "unexpected message: {message}"
        );
    }

    let path = temp_config("[http]\nmax_in_flight_requests = 1\n");
    let config = load_file(&path).expect("a cap of one must be accepted");
    std::fs::remove_file(&path).ok();
    assert_eq!(config.http.max_in_flight_requests, 1);
}

#[test]
fn non_loopback_binding_requires_explicit_origins() {
    let message = rejection("[http]\nhost = \"0.0.0.0\"\nallow_non_loopback = true\n");
    assert!(
        message.contains("http.allowed_origins"),
        "unexpected message: {message}"
    );
}

#[test]
fn non_loopback_binding_is_accepted_when_fully_configured() {
    accepted(
        "[http]\nhost = \"0.0.0.0\"\nallow_non_loopback = true\nrequire_auth = true\nallowed_origins = [\"https://example.com\"]\n",
    );
}

#[test]
fn localhost_is_recognised_as_loopback() {
    accepted("[http]\nhost = \"localhost\"\n");
    accepted("[http]\nhost = \"::1\"\n");
}

#[test]
fn insecure_api_base_is_rejected() {
    let message = rejection("[discord]\napi_base = \"http://discord.com/api\"\n");
    assert!(
        message.contains("discord.api_base"),
        "unexpected message: {message}"
    );
}

#[test]
fn insecure_loopback_api_base_requires_the_test_flag() {
    let message = rejection("[discord]\napi_base = \"http://127.0.0.1:8090/api\"\n");
    assert!(
        message.contains("discord.api_base"),
        "unexpected message: {message}"
    );

    accepted(
        "[discord]\napi_base = \"http://127.0.0.1:8090/api\"\nallow_insecure_transport = true\n",
    );
}

#[test]
fn api_version_is_pinned_to_v10() {
    let message = rejection("[discord]\napi_version = \"v9\"\n");
    assert!(
        message.contains("discord.api_version"),
        "unexpected message: {message}"
    );
}

#[test]
fn trailing_slash_in_api_base_is_rejected() {
    let message = rejection("[discord]\napi_base = \"https://discord.com/api/\"\n");
    assert!(
        message.contains("discord.api_base"),
        "unexpected message: {message}"
    );
}

#[test]
fn token_variable_must_be_an_environment_name() {
    let message = rejection("[discord]\ntoken_env = \"not a name!\"\n");
    assert!(
        message.contains("discord.token_env"),
        "unexpected message: {message}"
    );
}

#[test]
fn allowlists_accept_only_snowflake_ids() {
    for entry in ["general", "000123456789012345", ""] {
        let message = rejection(&format!("[policy]\nchannel_allowlist = [\"{entry}\"]\n"));
        assert!(
            message.contains("policy.channel_allowlist"),
            "`{entry}` should be rejected: {message}"
        );
    }

    accepted("[policy]\nchannel_allowlist = [\"1234567890123456789\"]\n");
}

#[test]
fn read_and_write_limits_are_bounded() {
    let too_many_messages = rejection("[policy]\nmax_messages_per_read = 101\n");
    assert!(
        too_many_messages.contains("policy.max_messages_per_read"),
        "unexpected message: {too_many_messages}"
    );

    let too_long = rejection("[policy]\nmax_message_characters = 2_001\n");
    assert!(
        too_long.contains("policy.max_message_characters"),
        "unexpected message: {too_long}"
    );

    let no_concurrency = rejection("[policy]\nmax_concurrent_requests = 0\n");
    assert!(
        no_concurrency.contains("policy.max_concurrent_requests"),
        "unexpected message: {no_concurrency}"
    );

    for (field, value) in [
        ("max_embeds_per_message", "11"),
        ("max_embed_characters", "6001"),
        ("max_attachments_per_message", "11"),
        ("max_attachment_bytes", "0"),
        ("max_attachment_total_bytes", "0"),
    ] {
        let message = rejection(&format!("[policy]\n{field} = {value}\n"));
        assert!(
            message.contains(&format!("policy.{field}")),
            "unexpected message: {message}"
        );
    }

    let empty_root = rejection("[policy]\nattachment_root = \"  \"\n");
    assert!(
        empty_root.contains("policy.attachment_root"),
        "unexpected message: {empty_root}"
    );
    accepted("[policy]\nattachment_root = \"uploads\"\n");
}

#[test]
fn unknown_fields_are_rejected() {
    let message = rejection("[policy]\nsneaky_override = true\n");
    assert!(
        message.contains("unknown field"),
        "unexpected message: {message}"
    );
}

#[test]
fn invalid_log_levels_are_rejected() {
    let message = rejection("[logging]\nlevel = \"verbose\"\n");
    assert!(
        message.contains("logging.level"),
        "unexpected message: {message}"
    );
}

#[test]
fn unreadable_configuration_file_is_reported_with_its_path() {
    let path = std::env::temp_dir().join("discord-mcp-config-that-does-not-exist.toml");
    std::fs::remove_file(&path).ok();
    match load_file(&path) {
        Ok(_) => panic!("a missing file must not be accepted"),
        Err(error) => {
            let message = error.to_string();
            assert!(
                message.contains("discord-mcp-config-that-does-not-exist.toml"),
                "unexpected message: {message}"
            );
        }
    }
}

#[test]
fn control_tools_require_a_channel_allowlist() {
    let message = rejection(
        "[policy]
control_enabled = true
",
    );
    assert!(
        message.contains("policy.channel_allowlist"),
        "unexpected message: {message}"
    );
}

#[test]
fn administration_requires_a_guild_allowlist() {
    let message = rejection(
        "[policy]
administrative_enabled = true
",
    );
    assert!(
        message.contains("policy.guild_allowlist"),
        "unexpected message: {message}"
    );

    accepted(
        "[policy]
administrative_enabled = true
guild_allowlist = [\"123456789012345678\"]
",
    );
}

#[test]
fn moderation_requires_a_guild_allowlist() {
    let message = rejection(
        "[policy]
moderation_enabled = true
",
    );
    assert!(
        message.contains("policy.guild_allowlist"),
        "unexpected message: {message}"
    );

    accepted(
        "[policy]
moderation_enabled = true
guild_allowlist = [\"123456789012345678\"]
",
    );
}

#[test]
fn autonomy_limits_are_bounded() {
    for value in ["0", "601"] {
        let message = rejection(&format!(
            "[policy]
[policy.autonomy]
max_actions_per_minute = {value}
"
        ));
        assert!(
            message.contains("policy.autonomy.max_actions_per_minute"),
            "unexpected message: {message}"
        );
    }

    for value in ["0", "601"] {
        let message = rejection(&format!(
            "[policy]
[policy.autonomy]
max_sends_per_channel_per_minute = {value}
"
        ));
        assert!(
            message.contains("policy.autonomy.max_sends_per_channel_per_minute"),
            "unexpected message: {message}"
        );
    }

    for value in ["1", "101"] {
        let message = rejection(&format!(
            "[policy]
[policy.autonomy]
max_bulk_delete = {value}
"
        ));
        assert!(
            message.contains("policy.autonomy.max_bulk_delete"),
            "unexpected message: {message}"
        );
    }
}

#[test]
fn protected_ids_must_be_snowflakes() {
    for field in ["protected_channels", "protected_roles"] {
        let message = rejection(&format!(
            "[policy]
{field} = [\"not-an-id\"]
"
        ));
        assert!(
            message.contains(&format!("policy.{field}")),
            "unexpected message: {message}"
        );
    }
}

#[test]
fn an_empty_budget_state_file_is_rejected() {
    let message = rejection(
        "[policy]
[policy.autonomy]
budget_state_file = \"\"
",
    );
    assert!(
        message.contains("policy.autonomy.budget_state_file"),
        "unexpected message: {message}"
    );
}

#[test]
fn dry_run_entries_must_name_tools_and_be_unique() {
    let malformed = rejection(
        "[policy]
[policy.autonomy]
require_dry_run = [\"delete things\"]
",
    );
    assert!(
        malformed.contains("policy.autonomy.require_dry_run"),
        "unexpected message: {malformed}"
    );

    let duplicated = rejection(
        "[policy]
[policy.autonomy]
require_dry_run = [\"discord_delete_message\", \"discord_delete_message\"]
",
    );
    assert!(
        duplicated.contains("more than once"),
        "unexpected message: {duplicated}"
    );

    accepted(
        "[policy]
[policy.autonomy]
require_dry_run = [\"discord_delete_message\"]
",
    );
}

#[test]
fn mutation_classes_are_off_by_default() {
    let config = load(None).expect("the built-in defaults must load");

    assert!(!config.policy.control_enabled);
    assert!(!config.policy.moderation_enabled);
    assert!(!config.policy.administrative_enabled);
    assert_eq!(config.policy.autonomy.max_actions_per_minute, 30);
    assert_eq!(config.policy.autonomy.max_sends_per_channel_per_minute, 5);
    assert_eq!(config.policy.autonomy.max_bulk_delete, 50);
    assert!(config.policy.autonomy.require_dry_run.is_empty());
    assert_eq!(config.policy.autonomy.budget_state_file, None);
    assert!(config.policy.protected_channels.is_empty());
    assert!(config.policy.protected_roles.is_empty());
}
