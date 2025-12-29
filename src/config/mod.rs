pub mod http;
pub mod logging;

use std::error::Error;
use std::fmt::Display;

use http::adapters::env::EnvHttpConfig;
use http::ports::{HttpConfig, HttpConfigProvider};
use logging::adapters::env::EnvLoggingConfig;
use logging::ports::{LoggingConfig, LoggingConfigProvider};

#[derive(Clone)]
pub struct Config {
    pub http: HttpConfig,
    pub logging: LoggingConfig,
}

impl Config {
    pub fn load() -> Result<Self, ConfigError> {
        let http: HttpConfig = EnvHttpConfig::load()?;
        let logging: LoggingConfig = EnvLoggingConfig::load()?;

        Ok(Self { http, logging })
    }
}

#[derive(Debug)]
pub enum ConfigError {
    Missing(&'static str),
    Invalid(&'static str),
}

impl Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Missing(v) => write!(f, "missing env var: {}", v),
            ConfigError::Invalid(v) => write!(f, "invalid value for: {}", v),
        }
    }
}

impl Error for ConfigError {}

impl From<&ConfigError> for ConfigError {
    fn from(err: &ConfigError) -> Self {
        match err {
            ConfigError::Missing(v) => ConfigError::Missing(v),
            ConfigError::Invalid(v) => ConfigError::Invalid(v),
        }
    }
}
