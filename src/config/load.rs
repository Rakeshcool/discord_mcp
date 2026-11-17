//! Configuration loading.
//!
//! Precedence, highest first:
//! 1. an explicit file passed with `--config`,
//! 2. the file named by [`CONFIG_PATH_ENV`],
//! 3. the built-in defaults in `config/default.toml`, compiled into the binary.
//!
//! A file that fails to parse or validate is a startup failure. Nothing falls
//! back to defaults once a file has been named.

use std::path::Path;

use crate::config::schema::Config;
use crate::error::ConfigError;

/// Environment variable naming an alternative configuration file.
pub const CONFIG_PATH_ENV: &str = "DISCORD_MCP_CONFIG";

/// The shipped defaults, compiled in so the binary works with no file present.
pub const EMBEDDED_DEFAULT: &str = include_str!("../../config/default.toml");

/// Where a loaded configuration came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigSource {
    /// A configuration file on disk.
    File(String),
    /// The built-in `config/default.toml`.
    EmbeddedDefault,
}

impl ConfigSource {
    /// Human-readable description for logs.
    pub fn as_str(&self) -> &str {
        match self {
            Self::File(path) => path,
            Self::EmbeddedDefault => "built-in defaults",
        }
    }
}

/// Load and validate configuration, with its provenance.
pub fn load_with_source(path: Option<&Path>) -> Result<(Config, ConfigSource), ConfigError> {
    match path {
        Some(path) => {
            let config = load_file(path)?;
            Ok((config, ConfigSource::File(path.display().to_string())))
        }
        None => {
            let config = parse("built-in defaults", EMBEDDED_DEFAULT)?;
            Ok((config, ConfigSource::EmbeddedDefault))
        }
    }
}

/// Load and validate configuration from `path`, or the built-in defaults.
pub fn load(path: Option<&Path>) -> Result<Config, ConfigError> {
    load_with_source(path).map(|(config, _)| config)
}

/// Load and validate a specific file.
pub fn load_file(path: &Path) -> Result<Config, ConfigError> {
    let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
        path: path.display().to_string(),
        source,
    })?;
    parse(&path.display().to_string(), &text)
}

/// Resolve the configuration path from the environment, if one is set.
pub fn path_from_env() -> Option<String> {
    std::env::var(CONFIG_PATH_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn parse(source: &str, text: &str) -> Result<Config, ConfigError> {
    let config: Config = toml::from_str(text).map_err(|error| ConfigError::Parse {
        path: source.to_string(),
        message: error.to_string(),
    })?;
    config.validate()?;
    Ok(config)
}
