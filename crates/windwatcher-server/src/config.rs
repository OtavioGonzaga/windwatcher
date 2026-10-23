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
    Hs256JwtAccessTokenCodec, Hs256SigningSecret, RegistrationPolicy, SessionPolicy,
    SqliteStorageOptions,
};

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
    let _jwt = resolve_jwt(raw)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use windwatcher::{AccessTokenClaims, AccessTokenCodec, AccessTokenError, SessionId, UserId};

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
