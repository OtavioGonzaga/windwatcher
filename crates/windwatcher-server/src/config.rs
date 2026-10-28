use std::{
    env, fmt, fs, io,
    path::{Path, PathBuf},
    time::Duration,
};

use figment::{
    Figment,
    providers::{Env, Format, Serialized, Toml},
};
use serde::{Deserialize, Deserializer, Serialize};
use windwatcher::{
    Hs256JwtAccessTokenCodec, Hs256SigningSecret, MessagePolicy, RegistrationPolicy, SessionPolicy,
    SqliteQueueOptions, SqliteStorageOptions,
};

use crate::{cli::LogFormat, error::AppError, paths::ResolvedPaths};

pub(crate) mod http;
pub(crate) mod realtime;
use http::RawHttp;
use realtime::RawRealtime;

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
    pub queue: Option<RawQueue>,
    pub auth: RawAuth,
    pub messages: RawMessages,
    pub http: RawHttp,
    pub realtime: RawRealtime,
    pub logging: RawLogging,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RawQueue {
    pub provider: String,
    pub concurrency: usize,
    pub lease_duration: DurationValue,
    pub lease_renew_interval: DurationValue,
    pub poll_interval: DurationValue,
    pub default_max_attempts: u32,
    pub retry_base_delay: DurationValue,
    pub retry_max_delay: DurationValue,
    pub max_payload_bytes: usize,
    pub completed_retention: DurationValue,
    pub maintenance_interval: DurationValue,
    pub shutdown_grace_period: DurationValue,
    pub sqlite: RawQueueSqlite,
}
impl Default for RawQueue {
    fn default() -> Self {
        Self {
            provider: "sqlite".into(),
            concurrency: 1,
            lease_duration: DurationValue("60s".into()),
            lease_renew_interval: DurationValue("20s".into()),
            poll_interval: DurationValue("1s".into()),
            default_max_attempts: 5,
            retry_base_delay: DurationValue("1s".into()),
            retry_max_delay: DurationValue("5m".into()),
            max_payload_bytes: 262_144,
            completed_retention: DurationValue("7d".into()),
            maintenance_interval: DurationValue("5m".into()),
            shutdown_grace_period: DurationValue("30s".into()),
            sqlite: RawQueueSqlite::default(),
        }
    }
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RawQueueSqlite {
    pub path: Option<PathBuf>,
    pub create_if_missing: Option<bool>,
    pub max_connections: Option<u32>,
    pub busy_timeout: Option<DurationValue>,
}

#[derive(Clone, Debug)]
pub struct ResolvedQueueConfig {
    pub options: SqliteQueueOptions,
    pub create_if_missing: bool,
    pub derived_path: bool,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RawStorage {
    pub sqlite: Option<RawSqlite>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RawMessages {
    pub max_content_bytes: usize,
}
impl Default for RawMessages {
    fn default() -> Self {
        Self {
            max_content_bytes: MessagePolicy::default().max_content_bytes(),
        }
    }
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
#[derive(Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RawJwt {
    pub issuer: String,
    pub secret: Option<String>,
    pub secret_file: Option<PathBuf>,
}

struct EnvJwtOverride {
    secret: Option<String>,
    secret_file: Option<PathBuf>,
}
impl fmt::Debug for RawJwt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RawJwt")
            .field("issuer", &self.issuer)
            .field("secret", &self.secret.as_ref().map(|_| "[REDACTED]"))
            .field("secret_file", &self.secret_file)
            .finish()
    }
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
    let config_file = discover_config_file(&config_file, is_explicit)?;
    let mut figment = Figment::from(Serialized::defaults(RawConfig::default()));
    if let Some(config_file) = config_file {
        figment = figment.merge(Toml::file(config_file));
    }
    let mut raw: RawConfig = figment
        .merge(
            Env::prefixed("WINDWATCHER__")
                .filter(|key| {
                    !key.as_str().eq_ignore_ascii_case("AUTH__JWT__SECRET")
                        && !key.as_str().eq_ignore_ascii_case("AUTH__JWT__SECRET_FILE")
                })
                .split("__"),
        )
        .extract()
        .map_err(|error| AppError::Config(error.to_string()))?;
    if let Some(override_) = environment_jwt_override()? {
        raw.auth.jwt.secret = override_.secret;
        raw.auth.jwt.secret_file = override_.secret_file;
    }
    Ok(raw)
}

fn discover_config_file(path: &Path, explicit: bool) -> Result<Option<PathBuf>, AppError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => match fs::metadata(path) {
            Ok(target) if target.is_file() => Ok(Some(path.to_path_buf())),
            Ok(_) => Err(AppError::Config(format!(
                "configuration path {} is not a regular file",
                path.display()
            ))),
            Err(error) => Err(AppError::Config(format!(
                "failed to inspect configuration file {}: {error}",
                path.display()
            ))),
        },
        Ok(metadata) if metadata.is_file() => Ok(Some(path.to_path_buf())),
        Ok(_) => Err(AppError::Config(format!(
            "configuration path {} is not a regular file",
            path.display()
        ))),
        Err(error) if error.kind() == io::ErrorKind::NotFound && !explicit => Ok(None),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Err(AppError::Config(format!(
            "configuration file {} does not exist",
            path.display()
        ))),
        Err(error) => Err(AppError::Config(format!(
            "failed to inspect configuration file {}: {error}",
            path.display()
        ))),
    }
}

fn environment_jwt_override() -> Result<Option<EnvJwtOverride>, AppError> {
    let secret = environment_utf8("WINDWATCHER__AUTH__JWT__SECRET")?;
    let secret_file = environment_utf8("WINDWATCHER__AUTH__JWT__SECRET_FILE")?;
    match (secret, secret_file) {
        (Some(secret), secret_file) => Ok(Some(EnvJwtOverride {
            secret: Some(secret),
            secret_file: secret_file.map(PathBuf::from),
        })),
        (None, Some(path)) => Ok(Some(EnvJwtOverride {
            secret: None,
            secret_file: Some(PathBuf::from(path)),
        })),
        (None, None) => Ok(None),
    }
}

fn environment_utf8(key: &str) -> Result<Option<String>, AppError> {
    match env::var(key) {
        Ok(value) => Ok(Some(value)),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(env::VarError::NotUnicode(_)) => Err(AppError::Config(format!(
            "environment variable {key} must be valid UTF-8"
        ))),
    }
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
    options
        .validate()
        .map_err(|error| AppError::Config(error.to_string()))?;
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
pub fn resolve_jwt(raw: &RawConfig) -> Result<Hs256JwtAccessTokenCodec, AppError> {
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
    let secret =
        Hs256SigningSecret::new(bytes).map_err(|error| AppError::Config(error.to_string()))?;
    Ok(Hs256JwtAccessTokenCodec::new(
        secret,
        raw.auth.jwt.issuer.clone(),
    ))
}

pub fn resolve_full(raw: &RawConfig, paths: &ResolvedPaths) -> Result<(), AppError> {
    resolve_sqlite(raw, paths)?;
    resolve_registration(raw)?;
    resolve_session(raw)?;
    raw.http.resolve().map_err(AppError::Config)?;
    raw.realtime.resolve().map_err(AppError::Config)?;
    if raw.queue.is_some() {
        resolve_queue(raw, paths)?;
    }
    let _messages = resolve_messages(raw);
    let _jwt = resolve_jwt(raw)?;
    Ok(())
}

pub fn resolve_queue(
    raw: &RawConfig,
    paths: &ResolvedPaths,
) -> Result<ResolvedQueueConfig, AppError> {
    let Some(queue) = raw.queue.as_ref() else {
        return Err(AppError::Config(
            "queue is not configured; add a [queue] section".into(),
        ));
    };
    if queue.provider != "sqlite" {
        return Err(AppError::Config("queue.provider must be `sqlite`".into()));
    }
    let lease_duration = queue.lease_duration.parse()?;
    let lease_renew_interval = queue.lease_renew_interval.parse()?;
    let poll_interval = queue.poll_interval.parse()?;
    let retry_base_delay = queue.retry_base_delay.parse()?;
    let retry_max_delay = queue.retry_max_delay.parse()?;
    let completed_retention = queue.completed_retention.parse()?;
    let maintenance_interval = queue.maintenance_interval.parse()?;
    let shutdown_grace_period = queue.shutdown_grace_period.parse()?;
    if queue.concurrency == 0
        || queue.default_max_attempts == 0
        || queue.max_payload_bytes == 0
        || lease_duration.is_zero()
        || lease_renew_interval.is_zero()
        || lease_renew_interval >= lease_duration
        || poll_interval.is_zero()
        || retry_base_delay.is_zero()
        || retry_max_delay < retry_base_delay
        || completed_retention.is_zero()
        || maintenance_interval.is_zero()
        || shutdown_grace_period.is_zero()
    {
        return Err(AppError::Config("queue durations, concurrency, payload limit and attempts must be positive; lease_renew_interval must be less than lease_duration and retry_max_delay must be at least retry_base_delay".into()));
    }
    let derived_path = queue.sqlite.path.is_none();
    let path = queue
        .sqlite
        .path
        .clone()
        .unwrap_or_else(|| paths.data_dir.join("queue.db"));
    let storage_location = raw
        .storage
        .sqlite
        .as_ref()
        .and_then(|s| s.url.as_deref())
        .map(PathBuf::from)
        .unwrap_or_else(|| paths.data_dir.join("windwatcher.db"));
    if same_database_path(&path, &storage_location) {
        return Err(AppError::Config("storage.sqlite and queue.sqlite resolve to the same database file; queue requires a separate database".into()));
    }
    let create_if_missing = queue.sqlite.create_if_missing.unwrap_or(true);
    let mut options = SqliteQueueOptions::new(path.to_string_lossy().to_string())
        .create_if_missing(create_if_missing);
    if let Some(connections) = queue.sqlite.max_connections {
        if connections == 0 {
            return Err(AppError::Config(
                "queue.sqlite.max_connections must be greater than zero".into(),
            ));
        }
        options = options.max_connections(connections);
    }
    if let Some(timeout) = &queue.sqlite.busy_timeout {
        let timeout = timeout.parse()?;
        if timeout.is_zero() {
            return Err(AppError::Config(
                "queue.sqlite.busy_timeout must be greater than zero".into(),
            ));
        }
        options = options.busy_timeout(timeout);
    }
    Ok(ResolvedQueueConfig {
        options,
        create_if_missing,
        derived_path,
    })
}

fn same_database_path(left: &Path, right: &Path) -> bool {
    fn normalized(path: &Path) -> PathBuf {
        let value = path.to_string_lossy();
        let value = value
            .strip_prefix("sqlite://")
            .or_else(|| value.strip_prefix("sqlite:"))
            .or_else(|| value.strip_prefix("file:"))
            .unwrap_or(&value);
        let value = value.split('?').next().unwrap_or(value);
        let path = PathBuf::from(value);
        if let Ok(canonical) = path.canonicalize() {
            return canonical;
        }
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        parent
            .canonicalize()
            .unwrap_or_else(|_| parent.to_path_buf())
            .join(path.file_name().unwrap_or_default())
    }
    normalized(left) == normalized(right)
}

pub fn resolve_messages(raw: &RawConfig) -> MessagePolicy {
    MessagePolicy::new(raw.messages.max_content_bytes)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use windwatcher::{AccessTokenClaims, AccessTokenCodec, AccessTokenError, SessionId, UserId};

    #[test]
    fn message_config_resolves_library_policy_without_io_or_arbitrary_bounds() {
        let raw = RawConfig::default();
        assert_eq!(resolve_messages(&raw), MessagePolicy::default());
        for maximum in [0, 4, 131_072, usize::MAX] {
            let mut raw = RawConfig::default();
            raw.messages.max_content_bytes = maximum;
            assert_eq!(resolve_messages(&raw).max_content_bytes(), maximum);
        }
        let raw: RawConfig = Figment::from(Serialized::defaults(RawConfig::default()))
            .merge(Toml::string("[messages]\nmax_content_bytes = 4"))
            .extract()
            .unwrap();
        let policy = resolve_messages(&raw);
        assert!(
            policy
                .validate(&windwatcher::MessageContent::new("😀").unwrap())
                .is_ok()
        );
        assert!(
            policy
                .validate(&windwatcher::MessageContent::new("😀a").unwrap())
                .is_err()
        );
        for invalid in [
            "[messages]\nunknown = true",
            "[messages]\nmax_content_bytes = -1",
            "[messages]\nmax_content_bytes = 'four'",
        ] {
            assert!(
                Figment::from(Serialized::defaults(RawConfig::default()))
                    .merge(Toml::string(invalid))
                    .extract::<RawConfig>()
                    .is_err()
            );
        }
    }

    #[test]
    fn jwt_secret_sources_are_exclusive_and_missing_secrets_are_rejected() {
        let raw = RawConfig::default();
        assert!(resolve_jwt(&raw).is_err());

        let mut raw = RawConfig::default();
        raw.auth.jwt.secret = Some("x".repeat(32));
        raw.auth.jwt.secret_file = Some(PathBuf::from("unused"));
        assert!(resolve_jwt(&raw).is_err());

        let mut raw = RawConfig::default();
        raw.auth.jwt.secret = Some("short".into());
        assert!(resolve_jwt(&raw).is_err());

        let mut raw = RawConfig::default();
        raw.auth.jwt.secret_file = Some(PathBuf::from("missing-jwt-secret"));
        assert!(resolve_jwt(&raw).is_err());
    }

    #[test]
    fn jwt_secret_files_preserve_exact_bytes() {
        let directory = tempfile::TempDir::new().unwrap();
        let path = directory.path().join("jwt.key");
        fs::write(&path, format!("{}\n", "x".repeat(32))).unwrap();
        let mut raw = RawConfig::default();
        raw.auth.jwt.secret_file = Some(path);
        let codec = resolve_jwt(&raw).unwrap();
        let now = time::OffsetDateTime::from_unix_timestamp(1).unwrap();
        let claims = AccessTokenClaims::new(
            UserId::new(),
            SessionId::new(),
            now,
            now + time::Duration::seconds(1),
        )
        .unwrap();
        let token = codec.issue(&claims).unwrap();
        let without_newline = Hs256JwtAccessTokenCodec::new(
            Hs256SigningSecret::new(vec![b'x'; 32]).unwrap(),
            "windwatcher".into(),
        );
        assert!(matches!(
            without_newline.verify(&token, now),
            Err(AccessTokenError::InvalidToken)
        ));
    }

    #[test]
    fn jwt_debug_output_never_contains_a_direct_secret() {
        let mut raw = RawConfig::default();
        raw.auth.jwt.secret = Some("super-secret-value".into());
        assert!(!format!("{raw:?}").contains("super-secret-value"));
    }

    #[test]
    fn semantic_auth_errors_are_separate_from_storage_resolution() {
        let mut raw = RawConfig::default();
        raw.auth.session.access_token_ttl = DurationValue("500ms".into());
        let paths = ResolvedPaths {
            config_file: PathBuf::from("unused"),
            data_dir: PathBuf::from("unused"),
        };
        assert!(resolve_sqlite(&raw, &paths).is_ok());
        assert!(resolve_session(&raw).is_err());
    }

    #[test]
    fn queue_configuration_is_optional_separate_and_validated() {
        let directory = tempfile::TempDir::new().unwrap();
        let paths = ResolvedPaths {
            config_file: PathBuf::from("unused"),
            data_dir: directory.path().to_path_buf(),
        };
        let mut raw = RawConfig::default();
        assert!(resolve_queue(&raw, &paths).is_err());
        raw.queue = Some(RawQueue::default());
        let resolved = resolve_queue(&raw, &paths).unwrap();
        assert!(resolved.derived_path);
        let _ = resolved.options;
        assert_eq!(
            directory.path().join("windwatcher.db"),
            paths.data_dir.join("windwatcher.db")
        );

        raw.queue.as_mut().unwrap().lease_renew_interval = DurationValue("60s".into());
        assert!(resolve_queue(&raw, &paths).is_err());
        raw.queue.as_mut().unwrap().lease_renew_interval = DurationValue("20s".into());
        raw.queue.as_mut().unwrap().sqlite.path = Some(paths.data_dir.join("windwatcher.db"));
        assert!(resolve_queue(&raw, &paths).is_err());
    }

    #[test]
    fn default_config_must_be_absent_or_a_regular_file() {
        let directory = tempfile::TempDir::new().unwrap();
        let missing = directory.path().join("missing.toml");
        assert!(discover_config_file(&missing, false).unwrap().is_none());
        assert!(discover_config_file(directory.path(), false).is_err());

        let link = directory.path().join("broken.toml");
        #[cfg(unix)]
        std::os::unix::fs::symlink(directory.path().join("missing-target"), &link).unwrap();
        #[cfg(unix)]
        assert!(discover_config_file(&link, false).is_err());
    }
}
