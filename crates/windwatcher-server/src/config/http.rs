use std::{num::NonZeroU32, time::Duration};

use axum::http::{HeaderValue, Uri};
use governor::Quota;
use serde::{Deserialize, Serialize};

use super::DurationValue;

/// HTTP behavior only; listener configuration belongs to the serving runtime.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RawHttp {
    /// Cross-origin browser access policy.
    pub cors: RawCors,
    /// Process-local client-IP quotas.
    pub rate_limit: RawRateLimit,
}

/// Empty origins deny cross-origin browser access by default.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RawCors {
    /// Explicit web origins, or a single `*` for any origin without credentials.
    pub allowed_origins: Vec<String>,
}

/// General API and additional authentication quotas.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RawRateLimit {
    /// Explicit proxy CIDRs allowed to supply X-Forwarded-For. Empty by default.
    pub trusted_proxies: Vec<String>,
    /// Whether client-IP quotas are enforced.
    pub enabled: bool,
    /// Quota shared by every API request from a resolved client IP.
    pub general: RawRateLimitPolicy,
    /// Additional quota shared by registration, login and refresh.
    pub auth: RawRateLimitPolicy,
}
impl Default for RawRateLimit {
    fn default() -> Self {
        Self {
            trusted_proxies: Vec::new(),
            enabled: true,
            general: RawRateLimitPolicy::default(),
            auth: RawRateLimitPolicy {
                requests: 10,
                period: "1m".into(),
                burst: 10,
            },
        }
    }
}

/// Token-bucket policy: sustained requests per period and initial/maximum burst.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RawRateLimitPolicy {
    /// Sustained request count, greater than zero.
    pub requests: u32,
    /// Positive human-readable duration, such as `1s` or `1m`.
    pub period: String,
    /// Maximum immediately available requests, greater than zero.
    pub burst: u32,
}
impl Default for RawRateLimitPolicy {
    fn default() -> Self {
        Self {
            requests: 100,
            period: "1s".into(),
            burst: 200,
        }
    }
}

/// Validated HTTP configuration, ready for router composition without further I/O.
#[derive(Clone, Debug)]
pub struct ResolvedHttpConfig {
    pub(crate) trusted_proxies: Vec<ipnet::IpNet>,
    pub(crate) origins: Vec<HeaderValue>,
    pub(crate) any_origin: bool,
    pub(crate) enabled: bool,
    pub(crate) general: Quota,
    pub(crate) auth: Quota,
}

impl RawHttp {
    /// Validates origins and quotas, even if rate limiting is disabled.
    pub fn resolve(&self) -> Result<ResolvedHttpConfig, String> {
        let any_origin = self.cors.allowed_origins.iter().any(|value| value == "*");
        if any_origin && self.cors.allowed_origins.len() != 1 {
            return Err("http.cors wildcard must be used alone".into());
        }
        let mut origins = Vec::new();
        if !any_origin {
            for value in &self.cors.allowed_origins {
                let invalid = || {
                    "http.cors.allowed_origins must contain web origins without paths, queries or userinfo".to_owned()
                };
                let uri: Uri = value.parse().map_err(|_| invalid())?;
                if !matches!(uri.scheme_str(), Some("http" | "https"))
                    || uri.host().is_none_or(str::is_empty)
                    || uri.authority().is_some_and(|a| a.as_str().contains('@'))
                    || uri.path_and_query().is_some_and(|p| p.as_str() != "/")
                    || value.ends_with('/')
                    || value.contains('#')
                {
                    return Err(invalid());
                }
                origins.push(HeaderValue::from_str(value).map_err(|_| invalid())?);
            }
        }
        Ok(ResolvedHttpConfig {
            trusted_proxies: self
                .rate_limit
                .trusted_proxies
                .iter()
                .map(|value| {
                    value
                        .parse::<ipnet::IpNet>()
                        .map(|network| network.trunc())
                        .map_err(|_| {
                            "http.rate_limit.trusted_proxies requires explicit IPv4 or IPv6 CIDRs"
                                .to_owned()
                        })
                })
                .collect::<Result<_, _>>()?,
            origins,
            any_origin,
            enabled: self.rate_limit.enabled,
            general: self.rate_limit.general.resolve("general")?,
            auth: self.rate_limit.auth.resolve("auth")?,
        })
    }
}

impl RawRateLimitPolicy {
    fn resolve(&self, name: &str) -> Result<Quota, String> {
        let invalid = || {
            format!(
                "http.rate_limit.{name} requires positive requests, burst and a representable period per request of at least 1ns"
            )
        };
        let requests = NonZeroU32::new(self.requests).ok_or_else(invalid)?;
        let burst = NonZeroU32::new(self.burst).ok_or_else(invalid)?;
        let period = DurationValue(self.period.clone())
            .parse()
            .map_err(|_| invalid())?;
        // Round upward so integer nanosecond precision never permits a faster rate.
        let nanos = period.as_nanos().div_ceil(u128::from(requests.get()));
        let nanos = u64::try_from(nanos).map_err(|_| invalid())?;
        let quota = Quota::with_period(Duration::from_nanos(nanos)).ok_or_else(invalid)?;
        // Governor's nanosecond state must also represent a completely full bucket.
        nanos
            .checked_mul(u64::from(burst.get()))
            .ok_or_else(invalid)?;
        Ok(quota.allow_burst(burst))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_trusted_proxy_cidrs_even_when_disabled() {
        let mut raw = RawHttp::default();
        assert!(raw.resolve().unwrap().trusted_proxies.is_empty());
        raw.rate_limit.enabled = false;
        for value in [
            "",
            "127.0.0.1",
            "localhost/32",
            "0.0.0.0/33",
            "::1/129",
            "http://127.0.0.1/32",
            "SENTINEL_PROXY_SECRET",
        ] {
            raw.rate_limit.trusted_proxies = vec![value.into()];
            let error = raw.resolve().unwrap_err();
            assert!(!error.contains(value) || value.is_empty());
        }
        raw.rate_limit.trusted_proxies = vec!["127.0.0.1/32".into(), "::1/128".into()];
        assert_eq!(raw.resolve().unwrap().trusted_proxies.len(), 2);
    }

    #[test]
    fn validates_origins_and_wildcard() {
        assert!(RawHttp::default().resolve().is_ok());
        for origins in [
            vec!["*"],
            vec!["https://example.com", "http://localhost:3000"],
        ] {
            let mut raw = RawHttp::default();
            raw.cors.allowed_origins = origins.into_iter().map(str::to_owned).collect();
            assert!(raw.resolve().is_ok());
        }
        for origins in [
            vec!["*", "https://example.com"],
            vec!["null"],
            vec!["https://example.com/path"],
            vec!["https://example.com/"],
            vec!["https://user@example.com"],
            vec!["https://example.com?secret=x"],
            vec!["https://example.com\r\nx: y"],
        ] {
            let mut raw = RawHttp::default();
            raw.cors.allowed_origins = origins.into_iter().map(str::to_owned).collect();
            assert!(raw.resolve().is_err());
        }
    }

    #[test]
    fn validates_quotas_even_when_disabled() {
        for (requests, burst, period) in [
            (0, 1, "1s"),
            (1, 0, "1s"),
            (1, 1, "0s"),
            (1, 1, "oops"),
            (1, u32::MAX, "1000000000s"),
        ] {
            let mut raw = RawHttp::default();
            raw.rate_limit.enabled = false;
            raw.rate_limit.general = RawRateLimitPolicy {
                requests,
                burst,
                period: period.into(),
            };
            assert!(raw.resolve().is_err());
        }
    }
}
