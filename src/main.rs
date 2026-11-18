//! Command-line entry point.
//!
//! The binary is a thin wrapper around [`discord_mcp::app::service::AppService`].
//! Every diagnostic, including `--help` and `--version` output, goes to stderr:
//! stdout belongs to the MCP transport, and keeping it empty everywhere means
//! there is no mode in which a stray `println!` can corrupt a session.

use std::path::PathBuf;
use std::process::ExitCode;

use discord_mcp::app::service::AppService;
use discord_mcp::config::Config;
use discord_mcp::config::load;
use discord_mcp::error::{AppError, ErrorCategory};

const USAGE: &str = "\
discord-mcp — secure Discord MCP server (official bot identity only)

USAGE:
    discord-mcp <COMMAND> [--config <PATH>] [--read-only]

COMMANDS:
    serve                Run the MCP server over the configured transport
    --validate-config    Validate configuration, report, and exit
    --read-only          Disable every mutation class, whatever the config says
    --version, -V        Print the version
    --help, -h           Print this help

ENVIRONMENT:
    DISCORD_BOT_TOKEN       Bot token, required by `serve`
    DISCORD_MCP_CONFIG      Path to a TOML configuration file
    DISCORD_MCP_READ_ONLY   Set to 1 to disable every mutation class
    RUST_LOG                Log filter override

EXIT CODES:
    0   success
    1   internal failure
    2   usage or configuration error
    3   authentication failure
    4   policy refused the operation

stdout is reserved for MCP protocol frames; all output here goes to stderr.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Command {
    Serve,
    ValidateConfig,
    Version,
    Help,
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("discord-mcp: {error}");
            exit_code_for(error.category())
        }
    }
}

fn run() -> Result<ExitCode, AppError> {
    let mut command: Option<Command> = None;
    let mut config_path: Option<PathBuf> = None;
    let mut read_only = std::env::var("DISCORD_MCP_READ_ONLY")
        .map(|value| matches!(value.trim(), "1" | "true" | "yes"))
        .unwrap_or(false);

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let parsed = match arg.as_str() {
            "serve" => Command::Serve,
            "--validate-config" => Command::ValidateConfig,
            "--version" | "-V" => Command::Version,
            "--help" | "-h" => Command::Help,
            "--read-only" => {
                read_only = true;
                continue;
            }
            "--config" => {
                let value = args
                    .next()
                    .ok_or_else(|| AppError::invalid_argument("`--config` requires a path"))?;
                set_config_path(&mut config_path, &value)?;
                continue;
            }
            other if other.starts_with("--config=") => {
                let value = other.trim_start_matches("--config=");
                set_config_path(&mut config_path, value)?;
                continue;
            }
            other => {
                return Err(AppError::invalid_argument(format!(
                    "unknown argument `{other}`; see --help"
                )));
            }
        };

        if command.is_some() {
            return Err(AppError::invalid_argument(
                "exactly one command may be given; see --help",
            ));
        }
        command = Some(parsed);
    }

    let Some(command) = command else {
        eprintln!("{USAGE}");
        return Err(AppError::invalid_argument("no command given"));
    };

    match command {
        Command::Help => {
            eprintln!("{USAGE}");
            Ok(ExitCode::SUCCESS)
        }
        Command::Version => {
            eprintln!("discord-mcp {}", env!("CARGO_PKG_VERSION"));
            Ok(ExitCode::SUCCESS)
        }
        Command::ValidateConfig => {
            let path = resolve_config_path(config_path);
            let (mut config, source) = load::load_with_source(path.as_deref())?;
            if read_only {
                apply_read_only(&mut config);
                eprintln!("read-only override: every mutation class is disabled");
            }
            let report = AppService::validate_only(&config);
            eprintln!("configuration valid (source: {})", source.as_str());
            eprintln!(
                "transport: {}; bot token: {}",
                config.server.transport.as_str(),
                if report.token_present {
                    "present"
                } else {
                    "not set"
                }
            );
            for warning in &report.warnings {
                eprintln!("warning: {warning}");
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Serve => {
            let path = resolve_config_path(config_path);
            let mut config = load::load(path.as_deref())?;
            if read_only {
                apply_read_only(&mut config);
            }
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .map_err(|error| {
                    AppError::internal(format!("failed to start the async runtime: {error}"))
                })?;
            // The Discord client registers its rate-limit state with the Tokio
            // reactor, so bootstrap must run inside it.
            let service = runtime.block_on(async { AppService::bootstrap(config) })?;
            runtime.block_on(service.run())?;
            drop(runtime);
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// Turn every mutation class off without touching the configuration file.
///
/// This is the runtime kill switch: an operator who wants a read-only session
/// passes `--read-only` or sets `DISCORD_MCP_READ_ONLY=1`, and neither the
/// configuration nor the allowlists need to change.
fn apply_read_only(config: &mut Config) {
    config.policy.write_enabled = false;
    config.policy.control_enabled = false;
    config.policy.administrative_enabled = false;
}

fn resolve_config_path(explicit: Option<PathBuf>) -> Option<PathBuf> {
    explicit.or_else(|| load::path_from_env().map(PathBuf::from))
}

fn set_config_path(slot: &mut Option<PathBuf>, value: &str) -> Result<(), AppError> {
    if value.trim().is_empty() {
        return Err(AppError::invalid_argument(
            "`--config` path must not be empty",
        ));
    }
    if slot.is_some() {
        return Err(AppError::invalid_argument(
            "`--config` was given more than once",
        ));
    }
    *slot = Some(PathBuf::from(value));
    Ok(())
}

fn exit_code_for(category: ErrorCategory) -> ExitCode {
    match category {
        ErrorCategory::Configuration | ErrorCategory::InvalidArgument => ExitCode::from(2),
        ErrorCategory::Authentication => ExitCode::from(3),
        ErrorCategory::Permission | ErrorCategory::PolicyBlocked => ExitCode::from(4),
        _ => ExitCode::from(1),
    }
}
