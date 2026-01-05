use crate::{
    cli::{Cli, database::DatabaseCli},
    config::{
        ConfigError,
        database::ports::{DatabaseConfig, DatabaseConfigProvider},
    },
};
use clap::Parser;
use regex::Regex;

pub struct CliDatabaseConfig();

impl DatabaseConfigProvider for CliDatabaseConfig {
    fn load() -> Result<DatabaseConfig, ConfigError> {
        let args: DatabaseCli = Cli::parse_from(std::env::args_os()).database;

        let database_url: String = args
            .database_url
            .ok_or(ConfigError::Missing("database-url"))?;

        if !is_valid_jdbc_url(&database_url) {
            return Err(ConfigError::Invalid("database-url".into()));
        }

        Ok(DatabaseConfig { database_url })
    }
}

fn is_valid_jdbc_url(url: &str) -> bool {
    let pattern = r"^[a-zA-Z][a-zA-Z0-9+.-]*:\/\/
                    [^:\s\/]+        # usuário
                    :
                    [^@\s\/]+        # senha
                    @
                    [^:\s\/]+        # host
                    (:\d+)?           # porta opcional
                    \/
                    [^\s\/]+$        # database
                    ";

    let regex = Regex::new(pattern.replace(' ', "").as_str()).unwrap();
    regex.is_match(url)
}
