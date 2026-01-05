use crate::config::ConfigError;

#[derive(Clone, Debug)]
pub struct DatabaseConfig {
    pub database_url: String,
}

pub trait DatabaseConfigProvider {
    fn load() -> Result<DatabaseConfig, ConfigError>;
}
