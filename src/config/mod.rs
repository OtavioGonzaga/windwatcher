pub mod database;
pub mod http;
pub mod logging;

use std::error::Error;
use std::fmt::Display;

use database::adapters::{cli::CliDatabaseConfig, env::EnvDatabaseConfig};
use database::ports::{DatabaseConfig, DatabaseConfigProvider};
use http::adapters::{cli::CliHttpConfig, env::EnvHttpConfig};
use http::ports::{HttpConfig, HttpConfigProvider};
use logging::adapters::{cli::CliLoggingConfig, env::EnvLoggingConfig};
use logging::ports::{LoggingConfig, LoggingConfigProvider};

#[derive(Clone)]
pub struct Config {
    pub http: HttpConfig,
    pub logging: LoggingConfig,
    pub database: DatabaseConfig,
}

impl Config {
    pub fn load() -> Result<Self, ConfigError> {
        let http_configs: Vec<Result<HttpConfig, ConfigError>> =
            vec![CliHttpConfig::load(), EnvHttpConfig::load()];
        let logging_configs: Vec<Result<LoggingConfig, ConfigError>> =
            vec![CliLoggingConfig::load(), EnvLoggingConfig::load()];
        let database_configs: Vec<Result<DatabaseConfig, ConfigError>> =
            vec![CliDatabaseConfig::load(), EnvDatabaseConfig::load()];
        let database: DatabaseConfig =
            merge_database(database_configs).expect("Failed to load database configuration");

        let http: HttpConfig = merge_http(http_configs).expect("Failed to load HTTP configuration");
        let logging: LoggingConfig =
            merge_logging(logging_configs).expect("Failed to load logging configuration");

        Ok(Self {
            http,
            logging,
            database,
        })
    }
}

fn merge_database(
    configs: Vec<Result<DatabaseConfig, ConfigError>>,
) -> Result<DatabaseConfig, ConfigError> {
    let database_url: String;

    if let Some(Ok(cfg)) = configs.iter().find(|r| r.is_ok()) {
        database_url = cfg.database_url.clone();
    } else {
        return Err(ConfigError::Missing("Database configuration"));
    }

    Ok(DatabaseConfig { database_url })
}

fn merge_http(configs: Vec<Result<HttpConfig, ConfigError>>) -> Result<HttpConfig, ConfigError> {
    let port: u16;
    let host: String;

    if let Some(Ok(cfg)) = configs.iter().find(|r| r.is_ok()) {
        port = cfg.port;
        host = cfg.host.clone();
    } else {
        return Err(ConfigError::Missing("HTTP configuration"));
    }

    Ok(HttpConfig { host, port })
}

fn merge_logging(
    configs: Vec<Result<LoggingConfig, ConfigError>>,
) -> Result<LoggingConfig, ConfigError> {
    let level: String;

    if let Some(Ok(cfg)) = configs.iter().find(|r| r.is_ok()) {
        level = cfg.level.clone();
    } else {
        return Err(ConfigError::Missing("Logging configuration"));
    }

    Ok(LoggingConfig { level })
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
