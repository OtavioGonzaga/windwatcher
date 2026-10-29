use std::{
    net::SocketAddr,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

use super::DurationValue;
use crate::error::AppError;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct RawServer {
    pub bind: String,
    pub shutdown_grace_period: DurationValue,
    pub readiness_timeout: DurationValue,
}

impl Default for RawServer {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:8080".into(),
            shutdown_grace_period: DurationValue("30s".into()),
            readiness_timeout: DurationValue("2s".into()),
        }
    }
}

impl RawServer {
    pub(crate) fn resolve(&self) -> Result<(SocketAddr, Duration, Duration), AppError> {
        let bind = self.bind.parse().map_err(|_| {
            AppError::Config("server.bind must be a numeric IP address and port".into())
        })?;
        let grace = self.shutdown_grace_period.parse()?;
        let readiness = self.readiness_timeout.parse()?;
        for (name, duration) in [
            ("shutdown_grace_period", grace),
            ("readiness_timeout", readiness),
        ] {
            if duration.is_zero() || Instant::now().checked_add(duration).is_none() {
                return Err(AppError::Config(format!(
                    "server.{name} must be positive and within the supported clock range"
                )));
            }
        }
        Ok((bind, grace, readiness))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use figment::{
        Figment,
        providers::{Format, Toml},
    };

    #[test]
    fn safe_defaults_and_numeric_listeners() {
        let config = RawServer::default();
        assert_eq!(
            config.resolve().unwrap(),
            (
                "127.0.0.1:8080".parse().unwrap(),
                Duration::from_secs(30),
                Duration::from_secs(2)
            )
        );
        for bind in ["127.0.0.1:0", "0.0.0.0:8080", "[::1]:8080", "[::]:0"] {
            assert!(
                RawServer {
                    bind: bind.into(),
                    ..config.clone()
                }
                .resolve()
                .is_ok()
            );
        }
        for bind in ["localhost:8080", "127.0.0.1", "[::]:65536"] {
            assert!(
                RawServer {
                    bind: bind.into(),
                    ..config.clone()
                }
                .resolve()
                .is_err()
            );
        }
    }

    #[test]
    fn rejects_zero_and_unrepresentable_deadlines_and_unknown_fields() {
        for value in ["0s", "18446744073709551615s"] {
            for field in ["shutdown_grace_period", "readiness_timeout"] {
                let raw: RawServer = Figment::new()
                    .merge(Toml::string(&format!("{field} = '{value}'")))
                    .extract()
                    .unwrap();
                assert!(raw.resolve().is_err());
            }
        }
        assert!(
            Figment::new()
                .merge(Toml::string("unknown = 1"))
                .extract::<RawServer>()
                .is_err()
        );
    }
}
