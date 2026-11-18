//! Process-level protocol tests.
//!
//! These run the real binary. The contract under test: in every mode, stdout
//! carries MCP frames and nothing else, and startup fails closed.

mod http_client;

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

const BIN: &str = env!("CARGO_BIN_EXE_discord-mcp");
const TOKEN: &str = "MTAwMDAwMDAwMDAwMDAwMDAw.Ga4dEf.abcdefghijklmnopqrstuvwxyz0123456789AB";
const TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug)]
struct Outcome {
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
    timed_out: bool,
}

fn run(args: &[&str], stdin_data: Option<&str>, token: Option<&str>) -> Outcome {
    let mut command = Command::new(BIN);
    command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // The harness environment must not decide the outcome of these tests.
    for variable in [
        "DISCORD_BOT_TOKEN",
        "DISCORD_MCP_HTTP_TOKEN",
        "DISCORD_MCP_CONFIG",
        "RUST_LOG",
    ] {
        command.env_remove(variable);
    }
    if let Some(token) = token {
        command.env("DISCORD_BOT_TOKEN", token);
    }

    let mut child = command.spawn().expect("the server binary must exist");

    if let Some(data) = stdin_data {
        let mut stdin = child.stdin.take().expect("stdin pipe");
        stdin.write_all(data.as_bytes()).expect("write stdin");
    }
    // Dropping stdin closes the pipe: an MCP client's way of stopping a stdio
    // server.
    drop(child.stdin.take());

    let mut stdout_pipe = child.stdout.take().expect("stdout pipe");
    let mut stderr_pipe = child.stderr.take().expect("stderr pipe");
    let stdout_reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout_pipe.read_to_string(&mut text);
        text
    });
    let stderr_reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr_pipe.read_to_string(&mut text);
        text
    });

    let deadline = Instant::now() + TIMEOUT;
    let mut timed_out = false;
    let exit_code = loop {
        match child.try_wait().expect("poll the child process") {
            Some(status) => break status.code(),
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                timed_out = true;
                break None;
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    };

    Outcome {
        exit_code,
        stdout: stdout_reader.join().expect("stdout reader"),
        stderr: stderr_reader.join().expect("stderr reader"),
        timed_out,
    }
}

const INITIALIZE: &str = concat!(
    "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{",
    "\"protocolVersion\":\"2025-11-25\",\"capabilities\":{},",
    "\"clientInfo\":{\"name\":\"phase4-test\",\"version\":\"0.1.0\"}}}\n",
    "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n",
);

fn tool_call(id: u32, name: &str, arguments: &str) -> String {
    format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"tools/call\",\"params\":{{\"name\":\"{name}\",\"arguments\":{arguments}}}}}\n"
    )
}

fn session(frames: &str) -> String {
    format!("{INITIALIZE}{frames}")
}

/// Run one stdio session against `config` and return every parsed response.
///
/// The frames are written at once and stdin is closed, which is how an MCP
/// client ends a session; the server must exit cleanly with protocol-only stdout.
fn run_session(name: &str, config: &str, frames: &str) -> Vec<Value> {
    let path = temp_config(name, config);
    let outcome = run(
        &["serve", "--config", &path.display().to_string()],
        Some(frames),
        Some(TOKEN),
    );
    std::fs::remove_file(&path).ok();

    assert!(
        !outcome.timed_out,
        "the server did not exit after stdin closed"
    );
    assert_eq!(outcome.exit_code, Some(0), "stderr: {}", outcome.stderr);
    assert!(
        !outcome.stderr.contains(TOKEN),
        "stderr leaked the bot token"
    );

    outcome
        .stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str(line)
                .unwrap_or_else(|error| panic!("stdout line is not JSON ({error}): {line:?}"))
        })
        .collect()
}

fn response_with_id(responses: &[Value], id: u64) -> &Value {
    responses
        .iter()
        .find(|value| value["id"] == id)
        .unwrap_or_else(|| panic!("no response with id {id}: {responses:?}"))
}

fn tool_error_text(response: &Value) -> &str {
    assert_eq!(
        response["result"]["isError"], true,
        "expected a tool error result, got {response}"
    );
    response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("no text content in {response}"))
}

fn temp_config(name: &str, text: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "discord-mcp-protocol-{}-{name}.toml",
        std::process::id()
    ));
    std::fs::write(&path, text).expect("write the temporary configuration");
    path
}

#[test]
fn validate_config_succeeds_and_keeps_stdout_clean() {
    let outcome = run(&["--validate-config"], None, None);

    assert!(!outcome.timed_out, "the process did not exit");
    assert_eq!(outcome.exit_code, Some(0), "stderr: {}", outcome.stderr);
    assert!(
        outcome.stdout.is_empty(),
        "stdout must stay clean, got: {:?}",
        outcome.stdout
    );
    assert!(
        outcome.stderr.contains("configuration valid"),
        "stderr: {}",
        outcome.stderr
    );
}

#[test]
fn invalid_configuration_fails_closed() {
    let path = temp_config(
        "write-without-allowlist",
        "[policy]\nwrite_enabled = true\n",
    );
    let outcome = run(
        &["--validate-config", "--config", &path.display().to_string()],
        None,
        None,
    );
    std::fs::remove_file(&path).ok();

    assert!(!outcome.timed_out, "the process did not exit");
    assert_eq!(outcome.exit_code, Some(2), "stderr: {}", outcome.stderr);
    assert!(outcome.stdout.is_empty(), "stdout must stay clean");
    assert!(
        outcome.stderr.contains("CONFIGURATION_ERROR"),
        "stderr: {}",
        outcome.stderr
    );
    assert!(
        outcome.stderr.contains("channel_allowlist"),
        "stderr: {}",
        outcome.stderr
    );
}

#[test]
fn serving_without_a_token_fails_closed() {
    let outcome = run(&["serve"], None, None);

    assert!(!outcome.timed_out, "the process did not exit");
    assert_eq!(outcome.exit_code, Some(2), "stderr: {}", outcome.stderr);
    assert!(outcome.stdout.is_empty(), "stdout must stay clean");
    assert!(
        outcome.stderr.contains("DISCORD_BOT_TOKEN"),
        "stderr: {}",
        outcome.stderr
    );
}

#[test]
fn unknown_arguments_are_rejected() {
    let outcome = run(&["--serve-everything"], None, None);

    assert!(!outcome.timed_out, "the process did not exit");
    assert_eq!(outcome.exit_code, Some(2));
    assert!(outcome.stdout.is_empty(), "stdout must stay clean");
    assert!(
        outcome.stderr.contains("unknown argument"),
        "stderr: {}",
        outcome.stderr
    );
}

#[test]
fn stdio_transport_writes_only_json_rpc_frames_to_stdout() {
    let frames = concat!(
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{",
        "\"protocolVersion\":\"2025-11-25\",\"capabilities\":{},",
        "\"clientInfo\":{\"name\":\"protocol-test\",\"version\":\"0.1.0\"}}}\n",
        "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\",\"params\":{}}\n",
    );

    let outcome = run(&["serve"], Some(frames), Some(TOKEN));

    assert!(
        !outcome.timed_out,
        "the server did not exit after stdin closed"
    );
    assert_eq!(outcome.exit_code, Some(0), "stderr: {}", outcome.stderr);
    assert!(
        !outcome.stderr.contains(TOKEN),
        "stderr leaked the bot token"
    );

    let mut responses = Vec::new();
    for line in outcome
        .stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
    {
        let value: Value = serde_json::from_str(line)
            .unwrap_or_else(|error| panic!("stdout line is not JSON ({error}): {line:?}"));
        assert_eq!(value["jsonrpc"], "2.0", "not a JSON-RPC frame: {line}");
        responses.push(value);
    }

    let initialize = responses
        .iter()
        .find(|value| value["id"] == 1)
        .expect("an initialize response");
    assert_eq!(initialize["result"]["serverInfo"]["name"], "discord-mcp");
    assert!(
        initialize["result"]["protocolVersion"].is_string(),
        "missing negotiated protocol version: {initialize}"
    );
}

#[test]
fn a_disabled_read_returns_a_policy_blocked_tool_error() {
    let frames = session(&tool_call(2, "discord_list_guilds", "{}"));
    let responses = run_session("read-disabled", "[policy]\nread_enabled = false\n", &frames);

    let text = tool_error_text(response_with_id(&responses, 2));
    assert!(text.starts_with("POLICY_BLOCKED"), "got {text:?}");
}

#[test]
fn an_unlisted_guild_is_refused_at_the_tool_boundary() {
    let frames = session(&tool_call(2, "discord_get_guild", r#"{"guild_id":"2"}"#));
    let responses = run_session(
        "guild-allowlist",
        "[policy]\nguild_allowlist = [\"1\"]\n",
        &frames,
    );

    let text = tool_error_text(response_with_id(&responses, 2));
    assert!(text.starts_with("POLICY_BLOCKED"), "got {text:?}");
    assert!(text.contains("guild 2"), "got {text:?}");
    assert!(text.contains("guild_allowlist"), "got {text:?}");
}

#[test]
fn a_write_is_refused_while_writes_are_disabled() {
    let frames = session(&tool_call(
        2,
        "discord_send_message",
        r#"{"channel_id":"4","content":"hello"}"#,
    ));
    let responses = run_session("write-disabled", "", &frames);

    let text = tool_error_text(response_with_id(&responses, 2));
    assert!(text.starts_with("POLICY_BLOCKED"), "got {text:?}");
    assert!(text.contains("write_enabled"), "got {text:?}");
}

#[test]
fn a_write_to_an_unlisted_channel_is_refused_at_the_tool_boundary() {
    let frames = session(&tool_call(
        2,
        "discord_send_message",
        r#"{"channel_id":"5","content":"hello"}"#,
    ));
    let responses = run_session(
        "write-allowlist",
        "[policy]\nwrite_enabled = true\nchannel_allowlist = [\"4\"]\n",
        &frames,
    );

    let text = tool_error_text(response_with_id(&responses, 2));
    assert!(text.starts_with("POLICY_BLOCKED"), "got {text:?}");
    assert!(text.contains("channel 5"), "got {text:?}");
    assert!(text.contains("channel_allowlist"), "got {text:?}");
}

#[test]
fn an_oversized_message_page_is_an_invalid_argument_tool_error() {
    let frames = session(&tool_call(
        2,
        "discord_get_messages",
        r#"{"channel_id":"4","limit":5000}"#,
    ));
    let responses = run_session("page-limit", "", &frames);

    let text = tool_error_text(response_with_id(&responses, 2));
    assert!(text.starts_with("INVALID_ARGUMENT"), "got {text:?}");
}

#[test]
fn an_unknown_argument_is_a_tool_error_from_the_strict_schema() {
    let frames = session(&tool_call(2, "discord_list_guilds", r#"{"surprise":true}"#));
    let responses = run_session("unknown-argument", "", &frames);

    let text = tool_error_text(response_with_id(&responses, 2));
    assert!(
        text.contains("failed to deserialize parameters"),
        "got {text:?}"
    );
}

#[test]
fn tools_are_advertised_with_strict_schemas_and_honest_annotations() {
    let frames =
        session("{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\",\"params\":{}}\n");
    let responses = run_session("tools-list", "", &frames);

    let tools = response_with_id(&responses, 2)["result"]["tools"]
        .as_array()
        .expect("a tools array");
    let mut names: Vec<&str> = tools
        .iter()
        .map(|tool| tool["name"].as_str().expect("a tool name"))
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        vec![
            "discord_add_reaction",
            "discord_assign_role",
            "discord_bulk_delete_messages",
            "discord_clear_reactions",
            "discord_create_channel",
            "discord_create_invite",
            "discord_create_role",
            "discord_create_thread",
            "discord_delete_channel",
            "discord_delete_invite",
            "discord_delete_message",
            "discord_delete_permission_overwrite",
            "discord_delete_role",
            "discord_edit_channel",
            "discord_edit_message",
            "discord_edit_role",
            "discord_edit_thread",
            "discord_get_capabilities",
            "discord_get_channel",
            "discord_get_guild",
            "discord_get_member",
            "discord_get_message",
            "discord_get_messages",
            "discord_list_channels",
            "discord_list_guilds",
            "discord_list_members",
            "discord_pin_message",
            "discord_remove_reaction",
            "discord_remove_role",
            "discord_send_message",
            "discord_set_permission_overwrite",
            "discord_unpin_message",
        ]
    );

    for tool in tools {
        assert_eq!(
            tool["inputSchema"]["additionalProperties"], false,
            "schema must reject unknown arguments: {tool}"
        );
        let name = tool["name"].as_str().expect("a tool name");
        let is_read = name.starts_with("discord_get_") || name.starts_with("discord_list_");
        assert_eq!(
            tool["annotations"]["readOnlyHint"], is_read,
            "the read-only hint must match what the tool does: {tool}"
        );
    }
}

#[test]
fn pin_tools_name_the_permission_they_need() {
    let frames =
        session("{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\",\"params\":{}}\n");
    let responses = run_session("pin-permission", "", &frames);

    let tools = response_with_id(&responses, 2)["result"]["tools"]
        .as_array()
        .expect("a tools array");
    for name in ["discord_pin_message", "discord_unpin_message"] {
        let tool = tools
            .iter()
            .find(|tool| tool["name"].as_str() == Some(name))
            .unwrap_or_else(|| panic!("{name} must be advertised"));
        let description = tool["description"].as_str().expect("a description");
        assert!(
            description.contains("PIN_MESSAGES"),
            "{name} must name PIN_MESSAGES, got {description:?}"
        );
        assert!(
            !description.contains("MANAGE_MESSAGES"),
            "{name} must not claim MANAGE_MESSAGES, got {description:?}"
        );
    }
}

#[test]
fn bulk_delete_is_refused_when_administrative_tools_are_off() {
    let frames = session(&tool_call(
        2,
        "discord_bulk_delete_messages",
        r#"{"channel_id":"4","message_ids":["1100000000000000001","1100000000000000002"]}"#,
    ));
    let responses = run_session(
        "bulk-off",
        "[policy]\nwrite_enabled = true\nchannel_allowlist = [\"4\"]\n",
        &frames,
    );

    let text = tool_error_text(response_with_id(&responses, 2));
    assert!(text.starts_with("POLICY_BLOCKED"), "got {text:?}");
    assert!(text.contains("administrative"), "got {text:?}");
}

#[test]
fn delete_is_refused_when_control_tools_are_off() {
    let frames = session(&tool_call(
        2,
        "discord_delete_message",
        r#"{"channel_id":"4","message_id":"1100000000000000001"}"#,
    ));
    let responses = run_session(
        "delete-off",
        "[policy]\nwrite_enabled = true\nchannel_allowlist = [\"4\"]\n",
        &frames,
    );

    let text = tool_error_text(response_with_id(&responses, 2));
    assert!(text.starts_with("POLICY_BLOCKED"), "got {text:?}");
    assert!(text.contains("control"), "got {text:?}");
}

#[test]
fn creating_a_channel_is_refused_when_control_tools_are_off() {
    let frames = session(&tool_call(
        2,
        "discord_create_channel",
        r#"{"guild_id":"1","name":"ops"}"#,
    ));
    let responses = run_session(
        "create-channel-off",
        "[policy]\nwrite_enabled = true\nchannel_allowlist = [\"4\"]\n",
        &frames,
    );

    let text = tool_error_text(response_with_id(&responses, 2));
    assert!(text.starts_with("POLICY_BLOCKED"), "got {text:?}");
    assert!(text.contains("control"), "got {text:?}");
}

#[test]
fn role_creation_is_refused_when_administration_is_off() {
    let frames = session(&tool_call(
        2,
        "discord_create_role",
        r#"{"guild_id":"1","name":"mods"}"#,
    ));
    let responses = run_session(
        "create-role-off",
        "[policy]\nadministrative_enabled = false\n",
        &frames,
    );

    let text = tool_error_text(response_with_id(&responses, 2));
    assert!(text.starts_with("POLICY_BLOCKED"), "got {text:?}");
    assert!(text.contains("administrative"), "got {text:?}");
}

#[test]
fn a_role_cannot_grant_administrator() {
    let frames = session(&tool_call(
        2,
        "discord_create_role",
        r#"{"guild_id":"1","name":"mods","permissions":["ADMINISTRATOR"],"dry_run":true}"#,
    ));
    let responses = run_session(
        "create-role-admin",
        "[policy]\nadministrative_enabled = true\nguild_allowlist = [\"1\"]\n",
        &frames,
    );

    let text = tool_error_text(response_with_id(&responses, 2));
    assert!(text.starts_with("INVALID_ARGUMENT"), "got {text:?}");
    assert!(text.contains("ADMINISTRATOR"), "got {text:?}");
}

#[test]
fn role_names_over_the_documented_limit_are_refused_before_a_request() {
    let too_long = "x".repeat(101);
    let arguments = format!(r#"{{"guild_id":"1","name":"{too_long}","dry_run":true}}"#);
    let responses = run_session(
        "create-role-name-too-long",
        "[policy]\nadministrative_enabled = true\nguild_allowlist = [\"1\"]\n",
        &session(&tool_call(2, "discord_create_role", &arguments)),
    );

    let text = tool_error_text(response_with_id(&responses, 2));
    assert!(text.starts_with("INVALID_ARGUMENT"), "got {text:?}");
    assert!(text.contains("1-100"), "got {text:?}");

    let at_limit = "x".repeat(100);
    let arguments = format!(r#"{{"guild_id":"1","name":"{at_limit}","dry_run":true}}"#);
    let responses = run_session(
        "create-role-name-at-limit",
        "[policy]\nadministrative_enabled = true\nguild_allowlist = [\"1\"]\n",
        &session(&tool_call(2, "discord_create_role", &arguments)),
    );

    let result = &response_with_id(&responses, 2)["result"];
    assert_eq!(result["isError"], false, "{result}");
}

#[test]
fn a_dry_run_role_reports_the_plan_without_a_request() {
    let frames = session(&tool_call(
        2,
        "discord_create_role",
        r#"{"guild_id":"1","name":"mods","dry_run":true}"#,
    ));
    let responses = run_session(
        "dry-run-role",
        "[policy]\nadministrative_enabled = true\nguild_allowlist = [\"1\"]\n",
        &frames,
    );

    let result = &response_with_id(&responses, 2)["result"];
    assert_eq!(result["isError"], false, "{result}");
    let text = result["content"][0]["text"]
        .as_str()
        .expect("a text result");
    assert!(text.contains("\"dry_run\":true"), "got {text}");
    assert!(
        text.contains("discord_create_role"),
        "the plan must name the tool: {text}"
    );
}

#[test]
fn a_dry_run_thread_reports_the_plan_without_a_request() {
    let frames = session(&tool_call(
        2,
        "discord_create_thread",
        r#"{"channel_id":"4","name":"incident","dry_run":true}"#,
    ));
    let responses = run_session(
        "dry-run-thread",
        "[policy]\ncontrol_enabled = true\nchannel_allowlist = [\"4\"]\n",
        &frames,
    );

    let result = &response_with_id(&responses, 2)["result"];
    assert_eq!(result["isError"], false, "{result}");
    let text = result["content"][0]["text"]
        .as_str()
        .expect("a text result");
    assert!(text.contains("\"dry_run\":true"), "got {text}");
    assert!(
        text.contains("discord_create_thread"),
        "the plan must name the tool: {text}"
    );
}

#[test]
fn a_dry_run_delete_reports_the_plan_without_a_request() {
    let frames = session(&tool_call(
        2,
        "discord_delete_message",
        r#"{"channel_id":"4","message_id":"1100000000000000001","dry_run":true}"#,
    ));
    let responses = run_session(
        "dry-run-delete",
        "[policy]\ncontrol_enabled = true\nchannel_allowlist = [\"4\"]\n",
        &frames,
    );

    let result = &response_with_id(&responses, 2)["result"];
    assert_eq!(result["isError"], false, "{result}");
    let text = result["content"][0]["text"]
        .as_str()
        .expect("a text result");
    assert!(text.contains("\"dry_run\":true"), "got {text}");
    assert!(
        text.contains("discord_delete_message"),
        "the plan must name the tool: {text}"
    );
}

#[test]
fn the_guilds_resource_is_advertised() {
    let frames =
        session("{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"resources/list\",\"params\":{}}\n");
    let responses = run_session("resources-list", "", &frames);

    let resources = response_with_id(&responses, 2)["result"]["resources"]
        .as_array()
        .expect("a resources array");
    assert_eq!(resources.len(), 1, "{resources:?}");
    assert_eq!(resources[0]["uri"], "discord://guilds");
}

#[test]
fn a_disabled_read_blocks_the_guilds_resource() {
    let frames = session(
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"resources/read\",\"params\":{\"uri\":\"discord://guilds\"}}\n",
    );
    let responses = run_session(
        "resource-denied",
        "[policy]\nread_enabled = false\n",
        &frames,
    );

    let response = response_with_id(&responses, 2);
    assert!(
        response["error"].is_object(),
        "resources report failures as protocol errors: {response}"
    );
    let message = response["error"]["message"]
        .as_str()
        .unwrap_or_else(|| panic!("no error message in {response}"));
    assert!(message.contains("POLICY_BLOCKED"), "got {message:?}");
}

// ---------------------------------------------------------------------------
// Streamable HTTP transport
// ---------------------------------------------------------------------------

const HTTP_TOKEN: &str = "http-test-token-0123456789abcdef";

const INITIALIZE_REQUEST: &str = concat!(
    "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{",
    "\"protocolVersion\":\"2025-11-25\",\"capabilities\":{},",
    "\"clientInfo\":{\"name\":\"http-test\",\"version\":\"0.1.0\"}}}"
);

/// A running HTTP server, killed when the test ends.
struct HttpServer {
    child: Child,
    address: String,
    config_path: PathBuf,
}

impl Drop for HttpServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        std::fs::remove_file(&self.config_path).ok();
    }
}

fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
    let port = listener.local_addr().expect("local address").port();
    drop(listener);
    port
}

/// Start `serve` with the HTTP transport and wait until it accepts connections.
fn start_http_server(name: &str, extra_config: &str) -> HttpServer {
    let port = free_port();
    let config = format!(
        "[server]\ntransport = \"http\"\n[http]\nhost = \"127.0.0.1\"\nport = {port}\nrequire_auth = true\nauth_token_env = \"DISCORD_MCP_HTTP_TOKEN\"\n{extra_config}"
    );
    let config_path = temp_config(name, &config);

    let mut command = Command::new(BIN);
    command
        .args(["serve", "--config", &config_path.display().to_string()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for variable in ["DISCORD_MCP_CONFIG", "RUST_LOG"] {
        command.env_remove(variable);
    }
    command
        .env("DISCORD_BOT_TOKEN", TOKEN)
        .env("DISCORD_MCP_HTTP_TOKEN", HTTP_TOKEN);

    let child = command.spawn().expect("start the HTTP server");
    let address = format!("127.0.0.1:{port}");

    let deadline = Instant::now() + Duration::from_secs(20);
    while std::net::TcpStream::connect(&address).is_err() {
        assert!(
            Instant::now() < deadline,
            "the HTTP server never listened on {address}"
        );
        std::thread::sleep(Duration::from_millis(25));
    }

    HttpServer {
        child,
        address,
        config_path,
    }
}

fn authorization() -> String {
    format!("Bearer {HTTP_TOKEN}")
}

#[test]
fn an_unauthenticated_http_request_is_refused() {
    let server = start_http_server("http-no-auth", "");

    let response = http_client::post(&server.address, "/mcp", &[], INITIALIZE_REQUEST);

    assert_eq!(response.status, 401, "body: {}", response.body);
    assert!(
        response
            .header("www-authenticate")
            .unwrap_or_default()
            .contains("Bearer"),
        "a client must learn which scheme to use"
    );
    assert!(
        !response.body.contains("serverInfo"),
        "an unauthenticated request must not be served: {}",
        response.body
    );
}

#[test]
fn a_wrong_http_bearer_token_is_refused() {
    let server = start_http_server("http-wrong-token", "");

    let response = http_client::post(
        &server.address,
        "/mcp",
        &[("Authorization", "Bearer not-the-token-0123456789")],
        INITIALIZE_REQUEST,
    );

    assert_eq!(response.status, 401, "body: {}", response.body);
}

#[test]
fn a_foreign_host_header_is_refused() {
    let server = start_http_server("http-bad-host", "");

    let response = http_client::post(
        &server.address,
        "/mcp",
        &[
            ("Host", "rebind.example:1234"),
            ("Authorization", &authorization()),
        ],
        INITIALIZE_REQUEST,
    );

    assert_eq!(response.status, 403, "body: {}", response.body);
}

#[test]
fn a_foreign_origin_header_is_refused() {
    let server = start_http_server("http-bad-origin", "");

    let response = http_client::post(
        &server.address,
        "/mcp",
        &[
            ("Origin", "https://evil.example"),
            ("Authorization", &authorization()),
        ],
        INITIALIZE_REQUEST,
    );

    assert_eq!(response.status, 403, "body: {}", response.body);
}

#[test]
fn an_authenticated_initialize_is_served() {
    let server = start_http_server("http-authenticated", "");

    let response = http_client::post(
        &server.address,
        "/mcp",
        &[("Authorization", &authorization())],
        INITIALIZE_REQUEST,
    );

    assert_eq!(response.status, 200, "body: {}", response.body);

    // The transport frames an initialize response as an SSE event, so the
    // JSON-RPC message is the `data:` payload.
    let payloads = http_client::event_data(&response.body);
    let payload = payloads
        .last()
        .unwrap_or_else(|| panic!("no SSE payload in {:?}", response.body));
    let value: Value = serde_json::from_str(payload)
        .unwrap_or_else(|error| panic!("payload is not JSON ({error}): {payload:?}"));

    assert_eq!(value["id"], 1, "payload: {payload}");
    assert_eq!(
        value["result"]["serverInfo"]["name"], "discord-mcp",
        "payload: {payload}"
    );
}

const HTTP_NOTIFICATION: &str = "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}";
const HTTP_TOOLS_LIST: &str = "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/list\"}";
const UNKNOWN_SESSION: &str = "00000000-0000-0000-0000-000000000000";

/// Initialize and return the session id the server announced.
fn start_session(server: &HttpServer) -> String {
    let auth = authorization();
    let init = http_client::post(
        &server.address,
        "/mcp",
        &[("Authorization", &auth)],
        INITIALIZE_REQUEST,
    );
    assert_eq!(init.status, 200, "body: {}", init.body);
    init.header("mcp-session-id")
        .unwrap_or_else(|| panic!("initialize must announce a session id: {:?}", init.headers))
        .to_string()
}

#[test]
fn a_health_probe_is_authenticated_and_reports_status() {
    let server = start_http_server("http-health", "");
    let auth = authorization();

    let refused = http_client::get(&server.address, "/health", &[]);
    assert_eq!(refused.status, 401, "body: {}", refused.body);
    assert!(
        !refused.body.contains("\"ok\""),
        "an unauthenticated probe must learn nothing: {}",
        refused.body
    );

    let probe = http_client::get(&server.address, "/health", &[("Authorization", &auth)]);
    assert_eq!(probe.status, 200, "body: {}", probe.body);
    let value: Value = serde_json::from_str(&probe.body)
        .unwrap_or_else(|error| panic!("health must answer JSON ({error}): {}", probe.body));
    assert_eq!(value["status"], "ok");
    assert_eq!(value["server"], "discord-mcp");
    assert_eq!(value["transport"], "http");
    assert_eq!(
        value["version"],
        env!("CARGO_PKG_VERSION"),
        "the probe must report the running version"
    );
    assert!(
        !probe.body.contains("Discord") && !probe.body.contains("token"),
        "a probe must not describe the bot or its credentials: {}",
        probe.body
    );
}

#[test]
fn an_http_session_is_created_acknowledged_and_ended() {
    let server = start_http_server("http-session-lifecycle", "");
    let auth = authorization();
    let sid = start_session(&server);

    let notification = http_client::post(
        &server.address,
        "/mcp",
        &[("Authorization", &auth), ("Mcp-Session-Id", &sid)],
        HTTP_NOTIFICATION,
    );
    assert_eq!(
        notification.status, 202,
        "a notification is accepted without an answer: {}",
        notification.body
    );

    let list = http_client::post(
        &server.address,
        "/mcp",
        &[("Authorization", &auth), ("Mcp-Session-Id", &sid)],
        HTTP_TOOLS_LIST,
    );
    assert_eq!(list.status, 200, "body: {}", list.body);
    assert!(
        list.body.contains("discord_list_guilds") && list.body.contains("discord_get_message"),
        "the session must serve the read tools: {}",
        list.body
    );

    let ended = http_client::delete(
        &server.address,
        "/mcp",
        &[("Authorization", &auth), ("Mcp-Session-Id", &sid)],
    );
    assert_eq!(ended.status, 202, "body: {}", ended.body);

    let reused = http_client::post(
        &server.address,
        "/mcp",
        &[("Authorization", &auth), ("Mcp-Session-Id", &sid)],
        HTTP_TOOLS_LIST,
    );
    assert_eq!(
        reused.status, 404,
        "a terminated session must not be reusable: {}",
        reused.body
    );
    assert!(
        reused.body.to_lowercase().contains("session"),
        "the refusal must say what was missing: {}",
        reused.body
    );
}

#[test]
fn an_unknown_http_session_is_refused() {
    let server = start_http_server("http-unknown-session", "");
    let auth = authorization();

    let response = http_client::post(
        &server.address,
        "/mcp",
        &[
            ("Authorization", &auth),
            ("Mcp-Session-Id", UNKNOWN_SESSION),
        ],
        HTTP_TOOLS_LIST,
    );

    assert_eq!(response.status, 404, "body: {}", response.body);
    assert!(
        !response.body.contains("tools"),
        "a request under an unknown session must not be served: {}",
        response.body
    );
}

#[test]
fn an_http_event_stream_disconnect_leaves_the_server_usable() {
    let server = start_http_server("http-stream-disconnect", "");
    let auth = authorization();
    let sid = start_session(&server);

    let status = http_client::get_headers_then_disconnect(
        &server.address,
        "/mcp",
        &[
            ("Authorization", &auth),
            ("Accept", "text/event-stream"),
            ("Mcp-Session-Id", &sid),
        ],
    );
    assert_eq!(status, 200, "the standalone event stream must open");

    // The socket is gone. A probe on the same session and a brand-new session
    // must both still work: one aborted stream cannot wedge the listener.
    let probe = http_client::get(&server.address, "/health", &[("Authorization", &auth)]);
    assert_eq!(probe.status, 200, "body: {}", probe.body);
    let fresh = start_session(&server);
    assert_ne!(fresh, sid, "a new client gets its own session");
}

#[test]
fn a_non_loopback_bind_without_its_bearer_token_fails_closed() {
    let path = temp_config(
        "http-non-loopback-without-token",
        concat!(
            "[server]\ntransport = \"http\"\n",
            "[http]\nhost = \"0.0.0.0\"\nallow_non_loopback = true\n",
            "require_auth = true\nallowed_origins = [\"https://example.com\"]\n",
        ),
    );
    let outcome = run(
        &["serve", "--config", &path.display().to_string()],
        None,
        Some(TOKEN),
    );
    std::fs::remove_file(&path).ok();

    assert!(!outcome.timed_out, "the process did not exit");
    assert_eq!(outcome.exit_code, Some(2), "stderr: {}", outcome.stderr);
    assert!(outcome.stdout.is_empty(), "stdout must stay clean");
    assert!(
        outcome.stderr.contains("DISCORD_MCP_HTTP_TOKEN"),
        "stderr: {}",
        outcome.stderr
    );
}
