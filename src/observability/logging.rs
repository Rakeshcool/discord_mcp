//! Logging setup.
//!
//! Two invariants, both load-bearing for the STDIO transport:
//!
//! 1. every log record goes to **stderr**, through a
//!    [`RedactedStderr`](crate::security::redaction::RedactedStderr) writer, and
//! 2. panics are printed to stderr by an explicit hook.
//!
//! Nothing in this crate writes to stdout outside the MCP transport. The
//! `clippy::print_stdout` lint in `Cargo.toml` enforces that at build time.

use std::io::{self, IsTerminal};
use std::str::FromStr;

use tracing::level_filters::LevelFilter;
use tracing_subscriber::EnvFilter;

use crate::config::schema::{LOG_LEVELS, LoggingConfig};
use crate::error::AppError;
use crate::security::redaction::{RedactedStderr, Redactor};

/// Install the global tracing subscriber and the panic hook.
///
/// `RUST_LOG`, when set, overrides `logging.level`. Calling this twice returns
/// an error rather than silently keeping the first subscriber.
pub fn init(logging: &LoggingConfig, redactor: &Redactor) -> Result<(), AppError> {
    let level = LevelFilter::from_str(logging.level.trim()).map_err(|_| {
        AppError::configuration(format!(
            "invalid logging level `{}`; expected one of: {}",
            logging.level.trim(),
            LOG_LEVELS.join(", ")
        ))
    })?;

    let filter = EnvFilter::builder()
        .with_default_directive(level.into())
        .from_env_lossy();

    tracing_subscriber::fmt()
        .with_writer(RedactedStderr::new(redactor.clone()))
        .with_env_filter(filter)
        .with_ansi(io::stderr().is_terminal())
        .with_target(true)
        .with_level(true)
        .try_init()
        .map_err(|error| {
            AppError::internal(format!("failed to install the tracing subscriber: {error}"))
        })?;

    install_panic_hook(redactor.clone());
    Ok(())
}

/// Route panic messages to stderr, redacted.
///
/// The default hook also writes to stderr, but it does not know how to redact;
/// a panic payload built from request data must not carry a token or message
/// body into the log.
pub fn install_panic_hook(redactor: Redactor) {
    std::panic::set_hook(Box::new(move |info| {
        let message = redactor.redact(&format!("panic: {info}"));
        eprintln!("{message}");
    }));
}
