use crate::{
    cli::{Cli, logging::LoggingCli},
    config::{
        ConfigError,
        logging::ports::{LoggingConfig, LoggingConfigProvider},
    },
};
use clap::Parser;

pub struct CliLoggingConfig();

impl LoggingConfigProvider for CliLoggingConfig {
    fn load() -> Result<LoggingConfig, ConfigError> {
        let args: LoggingCli = Cli::parse_from(std::env::args_os()).logging;

        let level: String = args.log_level.ok_or(ConfigError::Missing("log-level"))?;

        match level.as_str() {
            "error" | "warn" | "info" | "debug" | "trace" => {}
            _ => return Err(ConfigError::Invalid("log-level")),
        }

        Ok(LoggingConfig { level })
    }
}
