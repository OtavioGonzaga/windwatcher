use crate::config::ConfigError;

#[derive(Clone)]
pub struct HttpConfig {
    pub host: String,
    pub port: u16,
}

pub trait HttpConfigProvider {
    fn load() -> Result<HttpConfig, ConfigError>;
}
