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
        let jwt_secret: String =
            std::env::var("JWT_SECRET").map_err(|_| ConfigError::Missing("JWT_SECRET"))?;

        if !is_valid_host(&host) {
            return Err(ConfigError::Invalid("HTTP_HOST"));
        }

        Ok(HttpConfig {
            host,
            port,
            jwt_secret,
        })
    }
}

fn is_valid_host(host: &str) -> bool {
    host.parse::<IpAddr>().is_ok()
}
