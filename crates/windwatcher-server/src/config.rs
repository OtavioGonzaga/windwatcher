use std::{fs, path::PathBuf, time::Duration};

use figment::{
    Figment,
    providers::{Env, Format, Serialized, Toml},
};
use serde::{Deserialize, Deserializer, Serialize};
use windwatcher::{Hs256SigningSecret, RegistrationPolicy, SessionPolicy, SqliteStorageOptions};

use crate::{cli::LogFormat, error::AppError, paths::ResolvedPaths};

#[derive(Clone, Debug, Serialize)]
pub struct DurationValue(String);
impl DurationValue {
    fn parse(&self) -> Result<Duration, AppError> {
        humantime::parse_duration(&self.0)
            .map_err(|error| AppError::Config(format!("invalid duration {:?}: {error}", self.0)))
    }
}
impl<'de> Deserialize<'de> for DurationValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        humantime::parse_duration(&value).map_err(serde::de::Error::custom)?;
        Ok(Self(value))
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RawConfig {
    pub storage: RawStorage,
    pub auth: RawAuth,
    pub logging: RawLogging,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RawStorage {
    pub sqlite: Option<RawSqlite>,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RawSqlite {
    pub url: Option<String>,
    pub create_if_missing: Option<bool>,
    pub max_connections: Option<u32>,
    pub busy_timeout: Option<DurationValue>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RawAuth {
    pub registration: String,
    pub session: RawSession,
    pub jwt: RawJwt,
}
impl Default for RawAuth {
    fn default() -> Self {
        Self {
            registration: "closed".into(),
            session: RawSession::default(),
            jwt: RawJwt::default(),
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RawSession {
    pub access_token_ttl: DurationValue,
    pub session_ttl: DurationValue,
}
impl Default for RawSession {
    fn default() -> Self {
        Self {
            access_token_ttl: DurationValue("15m".into()),
            session_ttl: DurationValue("30d".into()),
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RawJwt {
    pub issuer: String,
    pub secret: Option<String>,
    pub secret_file: Option<PathBuf>,
}
impl Default for RawJwt {
    fn default() -> Self {
        Self {
            issuer: "windwatcher".into(),
            secret: None,
            secret_file: None,
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RawLogging {
    pub filter: String,
    pub format: LogFormat,
}
impl Default for RawLogging {
    fn default() -> Self {
        Self {
            filter: "info".into(),
            format: LogFormat::Compact,
        }
    }
}

pub fn load(paths: &ResolvedPaths, explicit: Option<PathBuf>) -> Result<RawConfig, AppError> {
    let is_explicit = explicit.is_some();
    let config_file = explicit.unwrap_or_else(|| paths.config_file.clone());
    if is_explicit && !config_file.is_file() {
        return Err(AppError::Config(format!(
            "configuration file {} does not exist",
            config_file.display()
        )));
    }
    let mut figment = Figment::from(Serialized::defaults(RawConfig::default()));
    if config_file.is_file() {
        figment = figment.merge(Toml::file(&config_file));
    }
    figment
        .merge(Env::prefixed("WINDWATCHER__").split("__"))
        .extract()
        .map_err(|error| AppError::Config(error.to_string()))
}

pub fn resolve_sqlite(
    raw: &RawConfig,
    paths: &ResolvedPaths,
) -> Result<(SqliteStorageOptions, bool, bool), AppError> {
    let config = raw.storage.sqlite.as_ref();
    let derived = config.and_then(|sqlite| sqlite.url.as_ref()).is_none();
    let location = config
        .and_then(|sqlite| sqlite.url.clone())
        .unwrap_or_else(|| paths.data_dir.join("windwatcher.db").display().to_string());
    let mut options = SqliteStorageOptions::new(location);
    if let Some(sqlite) = config {
        if let Some(value) = sqlite.create_if_missing {
            options = options.create_if_missing(value);
        }
        if let Some(value) = sqlite.max_connections {
            if value == 0 {
                return Err(AppError::Config(
                    "storage.sqlite.max_connections must be greater than zero".into(),
                ));
            }
            options = options.max_connections(value);
        }
        if let Some(value) = &sqlite.busy_timeout {
            let value = value.parse()?;
            if value.is_zero() {
                return Err(AppError::Config(
                    "storage.sqlite.busy_timeout must be greater than zero".into(),
                ));
            }
            options = options.busy_timeout(value);
        }
    }
    let create_if_missing = config
        .and_then(|sqlite| sqlite.create_if_missing)
        .unwrap_or(true);
    Ok((options, derived, create_if_missing))
}

pub fn resolve_registration(raw: &RawConfig) -> Result<RegistrationPolicy, AppError> {
    match raw.auth.registration.as_str() {
        "open" => Ok(RegistrationPolicy::Open),
        "closed" => Ok(RegistrationPolicy::Closed),
        _ => Err(AppError::Config(
            "auth.registration must be `open` or `closed`".into(),
        )),
    }
}
pub fn resolve_session(raw: &RawConfig) -> Result<SessionPolicy, AppError> {
    let access = time::Duration::try_from(raw.auth.session.access_token_ttl.parse()?)
        .map_err(|_| AppError::Config("access token duration is out of range".into()))?;
    let session = time::Duration::try_from(raw.auth.session.session_ttl.parse()?)
        .map_err(|_| AppError::Config("session duration is out of range".into()))?;
    SessionPolicy::new(access, session).map_err(|error| AppError::Config(error.to_string()))
}
pub fn resolve_jwt(raw: &RawConfig) -> Result<(), AppError> {
    if raw.auth.jwt.issuer.is_empty() {
        return Err(AppError::Config("auth.jwt.issuer must not be empty".into()));
    }
    let bytes = match (&raw.auth.jwt.secret, &raw.auth.jwt.secret_file) {
        (Some(_), Some(_)) => {
            return Err(AppError::Config(
                "auth.jwt.secret and auth.jwt.secret_file are mutually exclusive".into(),
            ));
        }
        (Some(secret), None) => secret.as_bytes().to_vec(),
        (None, Some(path)) => fs::read(path).map_err(|error| {
            AppError::Config(format!(
                "failed to read JWT secret file {}: {error}",
                path.display()
            ))
        })?,
        (None, None) => return Err(AppError::Config("JWT signing secret is required".into())),
    };
    Hs256SigningSecret::new(bytes).map_err(|error| AppError::Config(error.to_string()))?;
    Ok(())
}

pub fn resolve_full(raw: &RawConfig, paths: &ResolvedPaths) -> Result<(), AppError> {
    resolve_sqlite(raw, paths)?;
    resolve_registration(raw)?;
    resolve_session(raw)?;
    resolve_jwt(raw)
}
