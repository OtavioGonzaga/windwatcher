use std::net::IpAddr;

use crate::config::{
    ConfigError,
    http::ports::{HttpConfig, HttpConfigProvider},
};

pub struct EnvHttpConfig;

impl HttpConfigProvider for EnvHttpConfig {
    fn load() -> Result<HttpConfig, ConfigError> {
        dotenvy::dotenv().unwrap();

        let host: String =
            std::env::var("HTTP_HOST").map_err(|_| ConfigError::Missing("HTTP_HOST"))?;
        let port: String =
            std::env::var("HTTP_PORT").map_err(|_| ConfigError::Missing("HTTP_PORT"))?;
        let port: u16 = port
            .parse()
            .map_err(|_| ConfigError::Invalid("HTTP_PORT"))?;
        let token_secret: String =
            std::env::var("TOKEN_SECRET").map_err(|_| ConfigError::Missing("TOKEN_SECRET"))?;
        let token_ttl: u64 = std::env::var("TOKEN_TTL")
            .map_err(|_| ConfigError::Missing("TOKEN_TTL"))?
            .parse()
            .map_err(|_| ConfigError::Invalid("REFRESH_TOKEN_TTL"))?;
        let refresh_token_ttl: u64 = std::env::var("REFRESH_TOKEN_TTL")
            .map_err(|_| ConfigError::Missing("REFRESH_TOKEN_TTL"))?
            .parse()
            .map_err(|_| ConfigError::Invalid("REFRESH_TOKEN_TTL"))?;

        if !is_valid_host(&host) {
            return Err(ConfigError::Invalid("HTTP_HOST"));
        }

        Ok(HttpConfig {
            host,
            port,
            token_secret,
            token_ttl,
            refresh_token_ttl,
        })
    }
}

fn is_valid_host(host: &str) -> bool {
    host.parse::<IpAddr>().is_ok()
}
