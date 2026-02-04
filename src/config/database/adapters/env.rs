use crate::config::{
    ConfigError,
    database::ports::{DatabaseConfig, DatabaseConfigProvider},
};

pub struct EnvDatabaseConfig;

impl DatabaseConfigProvider for EnvDatabaseConfig {
    fn load() -> Result<DatabaseConfig, ConfigError> {
        dotenvy::dotenv().ok();

        let database_url: String = std::env::var("DATABASE_URL")
            .map_err(|_| ConfigError::Missing("DATABASE_URL".into()))?;

        Ok(DatabaseConfig { database_url })
    }
}
