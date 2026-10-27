use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Process-local realtime transport policy. Durations use human-readable syntax.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RawRealtime {
    /// One-use ticket lifetime.
    pub ticket_ttl: String,
    /// Maximum queued events per socket.
    pub outgoing_buffer: usize,
    /// Native Ping interval.
    pub heartbeat_interval: String,
    /// Maximum wait for a matching Pong.
    pub heartbeat_timeout: String,
    /// Persistence-backed session validation interval.
    pub session_recheck_interval: String,
    /// Maximum concurrently running notification jobs; saturation drops new jobs.
    pub dispatch_concurrency: usize,
    /// Whole notification deadline, including projection and audience hydration.
    pub dispatch_timeout: String,
}

impl Default for RawRealtime {
    fn default() -> Self {
        Self {
            ticket_ttl: "30s".into(),
            outgoing_buffer: 32,
            heartbeat_interval: "30s".into(),
            heartbeat_timeout: "10s".into(),
            session_recheck_interval: "30s".into(),
            dispatch_concurrency: 32,
            dispatch_timeout: "5s".into(),
        }
    }
}

/// Validated realtime policy, consumed without configuration I/O.
#[derive(Clone, Debug)]
pub struct ResolvedRealtimeConfig {
    pub(crate) ticket_ttl: Duration,
    pub(crate) outgoing_buffer: usize,
    pub(crate) heartbeat_interval: Duration,
    pub(crate) heartbeat_timeout: Duration,
    pub(crate) session_recheck_interval: Duration,
    pub(crate) dispatch_concurrency: usize,
    pub(crate) dispatch_timeout: Duration,
}

impl RawRealtime {
    /// Rejects zero values, nonrepresentable deadlines and invalid heartbeat timing.
    pub fn resolve(&self) -> Result<ResolvedRealtimeConfig, String> {
        let duration = |name: &str, value: &str| {
            let duration =
                humantime::parse_duration(value).map_err(|_| format!("invalid realtime.{name}"))?;
            if duration.is_zero()
                || std::time::Instant::now().checked_add(duration).is_none()
                || time::Duration::try_from(duration).is_err()
            {
                return Err(format!(
                    "realtime.{name} must be positive and representable"
                ));
            }
            Ok(duration)
        };
        let config = ResolvedRealtimeConfig {
            ticket_ttl: duration("ticket_ttl", &self.ticket_ttl)?,
            outgoing_buffer: self.outgoing_buffer,
            heartbeat_interval: duration("heartbeat_interval", &self.heartbeat_interval)?,
            heartbeat_timeout: duration("heartbeat_timeout", &self.heartbeat_timeout)?,
            session_recheck_interval: duration(
                "session_recheck_interval",
                &self.session_recheck_interval,
            )?,
            dispatch_concurrency: self.dispatch_concurrency,
            dispatch_timeout: duration("dispatch_timeout", &self.dispatch_timeout)?,
        };
        if config.outgoing_buffer == 0
            || config.outgoing_buffer > tokio::sync::Semaphore::MAX_PERMITS
        {
            return Err(
                "realtime.outgoing_buffer must be positive and supported by the channel".into(),
            );
        }
        if config.heartbeat_timeout >= config.heartbeat_interval {
            return Err("realtime.heartbeat_timeout must be less than heartbeat_interval".into());
        }
        if config.dispatch_concurrency == 0
            || config.dispatch_concurrency > tokio::sync::Semaphore::MAX_PERMITS
        {
            return Err(
                "realtime.dispatch_concurrency must be positive and supported by the semaphore"
                    .into(),
            );
        }
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_and_rejections() {
        let defaults = RawRealtime::default();
        let config = defaults.resolve().unwrap();
        assert_eq!(config.ticket_ttl, Duration::from_secs(30));
        assert_eq!(config.outgoing_buffer, 32);
        for field in [
            "ticket_ttl",
            "heartbeat_interval",
            "heartbeat_timeout",
            "session_recheck_interval",
            "dispatch_timeout",
        ] {
            let mut value = serde_json::to_value(&defaults).unwrap();
            value[field] = "0s".into();
            assert!(
                serde_json::from_value::<RawRealtime>(value)
                    .unwrap()
                    .resolve()
                    .is_err()
            );
        }
        assert!(
            RawRealtime {
                outgoing_buffer: 0,
                ..defaults.clone()
            }
            .resolve()
            .is_err()
        );
        assert!(
            RawRealtime {
                heartbeat_timeout: "30s".into(),
                ..defaults
            }
            .resolve()
            .is_err()
        );
        assert!(
            RawRealtime {
                dispatch_concurrency: 0,
                ..RawRealtime::default()
            }
            .resolve()
            .is_err()
        );
    }
}
