use crate::config::{
    ConfigError,
    http::ports::{HttpConfig, HttpConfigProvider},
};

pub struct EnvHttpConfig;

impl HttpConfigProvider for EnvHttpConfig {
    fn load() -> Result<HttpConfig, ConfigError> {
        Ok(HttpConfig {
            host: std::env::var("HTTP_HOST").unwrap_or_else(|_| "127.0.0.1".into()),
            port: std::env::var("HTTP_PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(8080),
        })
    }
}
