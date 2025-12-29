use crate::config::{
    ConfigError,
    logging::ports::{LoggingConfig, LoggingConfigProvider},
};

pub struct EnvLoggingConfig;

impl LoggingConfigProvider for EnvLoggingConfig {
    fn load() -> Result<LoggingConfig, ConfigError> {
        let level: String =
            std::env::var("LOG_LEVEL").map_err(|_| ConfigError::Missing("LOG_LEVEL"))?;

        match level.as_str() {
            "error" | "warn" | "info" | "debug" | "trace" => {}
            _ => return Err(ConfigError::Invalid("LOG_LEVEL")),
        }

        Ok(LoggingConfig { level })
    }
}
