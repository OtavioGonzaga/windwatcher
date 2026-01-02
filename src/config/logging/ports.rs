use crate::config::ConfigError;

#[derive(Clone, Debug)]
pub struct LoggingConfig {
    pub level: String,
}

pub trait LoggingConfigProvider {
    fn load() -> Result<LoggingConfig, ConfigError>;
}
