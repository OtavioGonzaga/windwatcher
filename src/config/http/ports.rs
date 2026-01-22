use crate::config::ConfigError;

#[derive(Clone)]
pub struct HttpConfig {
    pub host: String,
    pub port: u16,
    pub token_secret: String,
    pub token_ttl: u64,
    pub refresh_token_ttl: u64,
}

pub trait HttpConfigProvider {
    fn load() -> Result<HttpConfig, ConfigError>;
}
