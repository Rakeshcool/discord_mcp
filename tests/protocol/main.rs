//! Process-level protocol tests.
//!
//! These run the real binary. The contract under test: in every mode, stdout
//! carries MCP frames and nothing else, and startup fails closed.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
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
    for variable in ["DISCORD_BOT_TOKEN", "DISCORD_MCP_CONFIG", "RUST_LOG"] {
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
