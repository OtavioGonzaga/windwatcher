use std::{fmt, future::Future};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rand_core::{OsRng, RngCore};
use sha2::{Digest as _, Sha256};
use time::{Duration, OffsetDateTime};
use zeroize::Zeroizing;

use crate::{IdentityStore, IdentityStoreError, SessionId, User, UserId};

/// A revocable, absolutely-expiring authenticated session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    id: SessionId,
    user_id: UserId,
    created_at: OffsetDateTime,
    expires_at: OffsetDateTime,
    revoked_at: Option<OffsetDateTime>,
}

/// An invalid session lifecycle invariant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SessionError {
    /// The expiration was not strictly after creation.
    #[error("session expiration must be after creation")]
    InvalidExpiration,
}

impl Session {
    /// Creates an active session with an absolute expiration.
    pub fn new(
        id: SessionId,
        user_id: UserId,
        created_at: OffsetDateTime,
        expires_at: OffsetDateTime,
    ) -> Result<Self, SessionError> {
        if expires_at <= created_at {
            return Err(SessionError::InvalidExpiration);
        }
        Ok(Self {
            id,
            user_id,
            created_at,
            expires_at,
            revoked_at: None,
        })
    }
    /// Reconstructs a session loaded from persistence, preserving its first
    /// revocation timestamp when present.
    pub fn restore(
        id: SessionId,
        user_id: UserId,
        created_at: OffsetDateTime,
        expires_at: OffsetDateTime,
        revoked_at: Option<OffsetDateTime>,
    ) -> Result<Self, SessionError> {
        let mut session = Self::new(id, user_id, created_at, expires_at)?;
        session.revoked_at = revoked_at;
        Ok(session)
    }
    /// Returns the session identifier.
    pub fn id(&self) -> SessionId {
        self.id
    }
    /// Returns the owning user identifier.
    pub fn user_id(&self) -> UserId {
        self.user_id
    }
    /// Returns when this session was created.
    pub fn created_at(&self) -> OffsetDateTime {
        self.created_at
    }
    /// Returns the absolute expiration instant.
    pub fn expires_at(&self) -> OffsetDateTime {
        self.expires_at
    }
    /// Returns when the session was first revoked.
    pub fn revoked_at(&self) -> Option<OffsetDateTime> {
        self.revoked_at
    }
    /// Returns whether the session has been revoked.
    pub fn is_revoked(&self) -> bool {
        self.revoked_at.is_some()
    }
    /// Returns whether the session has expired at `now`.
    pub fn is_expired(&self, now: OffsetDateTime) -> bool {
        now >= self.expires_at
    }
    /// Returns whether the session can be refreshed at `now`.
    pub fn is_active(&self, now: OffsetDateTime) -> bool {
        !self.is_revoked() && !self.is_expired(now)
    }
    /// Revokes the session once, retaining the first revocation instant.
    pub fn revoke(&mut self, now: OffsetDateTime) {
        if self.revoked_at.is_none() {
            self.revoked_at = Some(now);
        }
    }
}

/// Lifetimes used for sessions and issued access tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionPolicy {
    access_token_ttl: Duration,
    session_ttl: Duration,
}
/// A nonsensical session policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SessionPolicyError {
    /// A duration was zero or negative.
    #[error("session lifetimes must be positive")]
    NonPositiveDuration,
    /// The access-token lifetime cannot produce a valid whole-second claim.
    #[error("access-token lifetime must be at least one second")]
    AccessTokenTtlTooShort,
    /// Access tokens would outlive a session.
    #[error("access-token lifetime cannot exceed session lifetime")]
    AccessExceedsSession,
}
impl Default for SessionPolicy {
    fn default() -> Self {
        Self {
            access_token_ttl: Duration::minutes(15),
            session_ttl: Duration::days(30),
        }
    }
}
impl SessionPolicy {
    /// Validates and constructs an explicit policy.
    pub fn new(
        access_token_ttl: Duration,
        session_ttl: Duration,
    ) -> Result<Self, SessionPolicyError> {
        if access_token_ttl <= Duration::ZERO || session_ttl <= Duration::ZERO {
            return Err(SessionPolicyError::NonPositiveDuration);
        }
        if access_token_ttl < Duration::SECOND {
            return Err(SessionPolicyError::AccessTokenTtlTooShort);
        }
        if access_token_ttl > session_ttl {
            return Err(SessionPolicyError::AccessExceedsSession);
        }
        Ok(Self {
            access_token_ttl,
            session_ttl,
        })
    }
    /// Returns the access-token lifetime.
    pub fn access_token_ttl(&self) -> Duration {
        self.access_token_ttl
    }
    /// Returns the session lifetime.
    pub fn session_ttl(&self) -> Duration {
        self.session_ttl
    }
}

/// An encoded bearer access token. Debug output never contains its value.
pub struct AccessToken(Zeroizing<String>);
impl AccessToken {
    /// Constructs an opaque token from a codec-produced encoded value.
    pub fn new(value: String) -> Self {
        Self(Zeroizing::new(value))
    }
    /// Exposes the bearer value at a transport boundary. Do not log it.
    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}
impl fmt::Debug for AccessToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AccessToken([REDACTED])")
    }
}

/// Format-independent, minimal access-token claims.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessTokenClaims {
    user_id: UserId,
    session_id: SessionId,
    issued_at: OffsetDateTime,
    expires_at: OffsetDateTime,
}
impl AccessTokenClaims {
    /// Constructs claims at whole-second precision, rejecting an expiration that
    /// is not strictly after issuance after that normalization.
    pub fn new(
        user_id: UserId,
        session_id: SessionId,
        issued_at: OffsetDateTime,
        expires_at: OffsetDateTime,
    ) -> Result<Self, AccessTokenError> {
        let issued_at = OffsetDateTime::from_unix_timestamp(issued_at.unix_timestamp())
            .expect("an existing OffsetDateTime always has a representable Unix timestamp");
        let expires_at = OffsetDateTime::from_unix_timestamp(expires_at.unix_timestamp())
            .expect("an existing OffsetDateTime always has a representable Unix timestamp");
        if expires_at <= issued_at {
            return Err(AccessTokenError::InvalidClaims);
        }
        Ok(Self {
            user_id,
            session_id,
            issued_at,
            expires_at,
        })
    }
    /// Returns the authenticated user.
    pub fn user_id(&self) -> UserId {
        self.user_id
    }
    /// Returns the associated session.
    pub fn session_id(&self) -> SessionId {
        self.session_id
    }
    /// Returns the issuance instant.
    pub fn issued_at(&self) -> OffsetDateTime {
        self.issued_at
    }
    /// Returns the expiration instant.
    pub fn expires_at(&self) -> OffsetDateTime {
        self.expires_at
    }
}
/// Failures issuing or verifying an access token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum AccessTokenError {
    /// The encoded token is malformed, unsupported, or has an invalid signature.
    #[error("invalid access token")]
    InvalidToken,
    /// The token was valid but expired at the supplied instant.
    #[error("access token expired")]
    Expired,
    /// Its semantic claims were invalid.
    #[error("invalid access token claims")]
    InvalidClaims,
    /// The codec could not sign a token.
    #[error("access token issuance failed")]
    SigningFailed,
    /// The codec could not verify a token.
    #[error("access token verification failed")]
    VerificationFailed,
}
/// Synchronous, format-independent access-token signing and verification.
pub trait AccessTokenCodec: Sync {
    /// Issues a bearer token for valid claims.
    fn issue(&self, claims: &AccessTokenClaims) -> Result<AccessToken, AccessTokenError>;
    /// Verifies a bearer token using the explicit current time.
    fn verify(
        &self,
        token: &AccessToken,
        now: OffsetDateTime,
    ) -> Result<AccessTokenClaims, AccessTokenError>;
}

/// A 256-bit opaque refresh bearer token.
pub struct RefreshToken(Zeroizing<String>);
impl RefreshToken {
    /// Reconstructs a canonical base64url-without-padding 256-bit token.
    pub fn parse(value: String) -> Result<Self, RefreshTokenError> {
        let value = Zeroizing::new(value);
        if value.len() != 43 {
            return Err(RefreshTokenError::InvalidToken);
        }
        let decoded = Zeroizing::new(
            URL_SAFE_NO_PAD
                .decode(&value)
                .map_err(|_| RefreshTokenError::InvalidToken)?,
        );
        if decoded.len() != 32 || URL_SAFE_NO_PAD.encode(&decoded) != value.as_str() {
            return Err(RefreshTokenError::InvalidToken);
        }
        Ok(Self(value))
    }
    fn from_bytes(bytes: &[u8; 32]) -> Self {
        Self(Zeroizing::new(URL_SAFE_NO_PAD.encode(bytes)))
    }
    /// Exposes the bearer value at a transport boundary. Do not log it.
    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}
impl fmt::Debug for RefreshToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RefreshToken([REDACTED])")
    }
}
/// A fixed SHA-256 fingerprint of a refresh token suitable for persistence.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct RefreshTokenHash([u8; 32]);
impl RefreshTokenHash {
    /// Returns the binary SHA-256 digest for a storage adapter.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}
impl fmt::Debug for RefreshTokenHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RefreshTokenHash([REDACTED])")
    }
}
/// Refresh-token generation or input failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RefreshTokenError {
    /// The externally supplied token was malformed or non-canonical.
    #[error("invalid refresh token")]
    InvalidToken,
    /// The entropy source could not generate a token.
    #[error("refresh token generation failed")]
    RandomGenerationFailed,
}
/// Generates and fingerprints opaque refresh tokens.
pub trait RefreshTokenProvider: Sync {
    /// Generates an independent 256-bit refresh secret.
    fn generate(&self) -> Result<RefreshToken, RefreshTokenError>;
    /// Returns the deterministic SHA-256 fingerprint used by storage.
    fn hash(&self, token: &RefreshToken) -> RefreshTokenHash;
}
/// Refresh-token provider backed by the operating-system CSPRNG.
#[derive(Debug, Default)]
pub struct OsRefreshTokenProvider;
impl RefreshTokenProvider for OsRefreshTokenProvider {
    fn generate(&self) -> Result<RefreshToken, RefreshTokenError> {
        let mut bytes = Zeroizing::new([0; 32]);
        OsRng
            .try_fill_bytes(bytes.as_mut())
            .map_err(|_| RefreshTokenError::RandomGenerationFailed)?;
        Ok(RefreshToken::from_bytes(&bytes))
    }
    fn hash(&self, token: &RefreshToken) -> RefreshTokenHash {
        RefreshTokenHash(Sha256::digest(token.expose_secret().as_bytes()).into())
    }
}

/// Expected outcome of an atomic refresh-token compare-and-swap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshRotationResult {
    /// The replacement hash was persisted.
    Rotated,
    /// The hash/session state no longer permitted rotation.
    Rejected,
}
/// Normalized failures at session persistence boundary.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SessionStoreError {
    /// Creation was rejected because the persisted owner does not exist.
    #[error("session owner not found")]
    OwnerNotFound,
    /// Creation was rejected because the persisted owner is disabled.
    #[error("session owner disabled")]
    OwnerDisabled,
    /// A storage conflict prevented the operation.
    #[error("session storage conflict")]
    Conflict,
    /// Storage is temporarily unavailable.
    #[error("session storage unavailable")]
    Unavailable(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// Another operational storage failure occurred.
    #[error("unexpected session storage failure")]
    Unexpected(#[source] Box<dyn std::error::Error + Send + Sync>),
}
/// Application-driven asynchronous session persistence operations.
pub trait SessionStore: Sync {
    /// Atomically stores a session and its first refresh fingerprint, or neither.
    /// Atomically require a persisted, enabled owner at insertion time, including
    /// races after authentication. Return `OwnerNotFound` or `OwnerDisabled`
    /// according to that same atomic snapshot, not a subsequent identity lookup.
    /// Session IDs and refresh fingerprints must remain unique: collisions are
    /// `Conflict`, not owner rejections, and must not replace existing records.
    /// Rejection must persist neither a session nor its refresh fingerprint.
    fn create_session(
        &self,
        session: &Session,
        refresh_hash: &RefreshTokenHash,
    ) -> impl Future<Output = Result<(), SessionStoreError>> + Send;
    /// Finds a session by its current refresh-token fingerprint.
    fn find_by_refresh_token_hash(
        &self,
        refresh_hash: &RefreshTokenHash,
    ) -> impl Future<Output = Result<Option<Session>, SessionStoreError>> + Send;
    /// Atomically rotates only when the old hash is current and the session is active at `now`.
    fn rotate_refresh_token(
        &self,
        session_id: SessionId,
        expected_old_hash: &RefreshTokenHash,
        replacement_hash: &RefreshTokenHash,
        now: OffsetDateTime,
    ) -> impl Future<Output = Result<RefreshRotationResult, SessionStoreError>> + Send;
    /// Revokes a session only if it belongs to `user_id`; returns whether it existed.
    fn revoke_session_for_user(
        &self,
        user_id: UserId,
        session_id: SessionId,
        revoked_at: OffsetDateTime,
    ) -> impl Future<Output = Result<bool, SessionStoreError>> + Send;
    /// Revokes every session for a user without rewriting prior revocation timestamps.
    fn revoke_all_for_user(
        &self,
        user_id: UserId,
        revoked_at: OffsetDateTime,
    ) -> impl Future<Output = Result<(), SessionStoreError>> + Send;
}

/// Successful session establishment result. Debug output redacts both bearer tokens.
pub struct SessionGrant {
    session: Session,
    access_token: AccessToken,
    refresh_token: RefreshToken,
    access_token_expires_at: OffsetDateTime,
}
impl SessionGrant {
    /// Returns the persisted session.
    pub fn session(&self) -> &Session {
        &self.session
    }
    /// Returns the access-token expiration.
    pub fn access_token_expires_at(&self) -> OffsetDateTime {
        self.access_token_expires_at
    }
    /// Splits this grant into its session and bearer tokens.
    pub fn into_parts(self) -> (Session, AccessToken, RefreshToken) {
        (self.session, self.access_token, self.refresh_token)
    }
}
impl fmt::Debug for SessionGrant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionGrant")
            .field("session", &self.session)
            .field("access_token", &"[REDACTED]")
            .field("refresh_token", &"[REDACTED]")
            .field("access_token_expires_at", &self.access_token_expires_at)
            .finish()
    }
}

/// Failure creating a session.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CreateSessionError {
    /// The authenticated snapshot or persisted session owner is disabled.
    #[error("user disabled")]
    UserDisabled,
    /// The persisted owner disappeared after authentication.
    #[error("user not found")]
    UserNotFound,
    /// Session expiration cannot be represented by the time type.
    #[error("session expiration cannot be represented")]
    TimeOverflow,
    /// Refresh-token creation failed before persistence.
    #[error("refresh token generation failed")]
    RefreshTokenGeneration(#[source] RefreshTokenError),
    /// Access-token issuance failed before persistence.
    #[error("access token issuance failed")]
    AccessTokenIssuance(#[source] AccessTokenError),
    /// Persisting the new session failed.
    #[error("session storage failed")]
    Storage(#[source] SessionStoreError),
}
/// Creates a session for an already authenticated user.
pub struct CreateSession<'a, S, R, C> {
    store: &'a S,
    refresh_tokens: &'a R,
    codec: &'a C,
    policy: SessionPolicy,
}
impl<'a, S: SessionStore, R: RefreshTokenProvider, C: AccessTokenCodec> CreateSession<'a, S, R, C> {
    /// Wires the session ports and explicit policy.
    pub fn new(store: &'a S, refresh_tokens: &'a R, codec: &'a C, policy: SessionPolicy) -> Self {
        Self {
            store,
            refresh_tokens,
            codec,
            policy,
        }
    }
    /// Creates and persists a session, issuing bearer tokens only after all earlier work succeeds.
    pub async fn execute(
        &self,
        user: &User,
        now: OffsetDateTime,
    ) -> Result<SessionGrant, CreateSessionError> {
        if user.is_disabled() {
            return Err(CreateSessionError::UserDisabled);
        }
        let expires_at = now
            .checked_add(self.policy.session_ttl())
            .ok_or(CreateSessionError::TimeOverflow)?;
        let session = Session::new(SessionId::new(), user.id(), now, expires_at)
            .expect("positive policy creates valid session");
        let refresh_token = self
            .refresh_tokens
            .generate()
            .map_err(CreateSessionError::RefreshTokenGeneration)?;
        let refresh_hash = self.refresh_tokens.hash(&refresh_token);
        let access_expires = now
            .checked_add(self.policy.access_token_ttl())
            .ok_or(CreateSessionError::TimeOverflow)?
            .min(expires_at);
        let claims = AccessTokenClaims::new(user.id(), session.id(), now, access_expires)
            .expect("positive policy creates valid claims");
        let access_token_expires_at = claims.expires_at();
        let access_token = self
            .codec
            .issue(&claims)
            .map_err(CreateSessionError::AccessTokenIssuance)?;
        self.store
            .create_session(&session, &refresh_hash)
            .await
            .map_err(|error| match error {
                SessionStoreError::OwnerDisabled => CreateSessionError::UserDisabled,
                SessionStoreError::OwnerNotFound => CreateSessionError::UserNotFound,
                other => CreateSessionError::Storage(other),
            })?;
        Ok(SessionGrant {
            session,
            access_token,
            refresh_token,
            access_token_expires_at,
        })
    }
}

/// Failure refreshing a session.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RefreshSessionError {
    /// The refresh token is unknown, stale, expired, revoked, or belongs to a disabled user.
    #[error("invalid refresh token")]
    InvalidRefreshToken,
    /// Generating a replacement refresh token failed.
    #[error("refresh token generation failed")]
    RefreshTokenGeneration(#[source] RefreshTokenError),
    /// Issuing the replacement access token failed.
    #[error("access token issuance failed")]
    AccessTokenIssuance(#[source] AccessTokenError),
    /// A session persistence operation failed.
    #[error("session storage failed")]
    Storage(#[source] SessionStoreError),
    /// Loading current user state failed.
    #[error("identity storage failed")]
    IdentityStorage(#[source] IdentityStoreError),
    /// Access-token expiration cannot be represented by the time type.
    #[error("access token expiration cannot be represented")]
    TimeOverflow,
}
/// Rotates a valid refresh token and issues a replacement access token.
pub struct RefreshSession<'a, S, I, R, C> {
    store: &'a S,
    identities: &'a I,
    refresh_tokens: &'a R,
    codec: &'a C,
    policy: SessionPolicy,
}
impl<'a, S: SessionStore, I: IdentityStore, R: RefreshTokenProvider, C: AccessTokenCodec>
    RefreshSession<'a, S, I, R, C>
{
    /// Wires the session, identity, and token ports.
    pub fn new(
        store: &'a S,
        identities: &'a I,
        refresh_tokens: &'a R,
        codec: &'a C,
        policy: SessionPolicy,
    ) -> Self {
        Self {
            store,
            identities,
            refresh_tokens,
            codec,
            policy,
        }
    }
    /// Rotates a current refresh token without extending session expiration.
    pub async fn execute(
        &self,
        refresh_token: &RefreshToken,
        now: OffsetDateTime,
    ) -> Result<SessionGrant, RefreshSessionError> {
        let old_hash = self.refresh_tokens.hash(refresh_token);
        let session = self
            .store
            .find_by_refresh_token_hash(&old_hash)
            .await
            .map_err(RefreshSessionError::Storage)?
            .filter(|s| s.is_active(now))
            .ok_or(RefreshSessionError::InvalidRefreshToken)?;
        let user = self
            .identities
            .find_user(session.user_id())
            .await
            .map_err(RefreshSessionError::IdentityStorage)?
            .filter(|u| !u.is_disabled())
            .ok_or(RefreshSessionError::InvalidRefreshToken)?;
        let replacement = self
            .refresh_tokens
            .generate()
            .map_err(RefreshSessionError::RefreshTokenGeneration)?;
        let replacement_hash = self.refresh_tokens.hash(&replacement);
        let access_expires = now
            .checked_add(self.policy.access_token_ttl())
            .ok_or(RefreshSessionError::TimeOverflow)?
            .min(session.expires_at());
        let claims = AccessTokenClaims::new(user.id(), session.id(), now, access_expires)
            .map_err(|_| RefreshSessionError::InvalidRefreshToken)?;
        let access_token_expires_at = claims.expires_at();
        let access_token = self
            .codec
            .issue(&claims)
            .map_err(RefreshSessionError::AccessTokenIssuance)?;
        match self
            .store
            .rotate_refresh_token(session.id(), &old_hash, &replacement_hash, now)
            .await
            .map_err(RefreshSessionError::Storage)?
        {
            RefreshRotationResult::Rotated => Ok(SessionGrant {
                session,
                access_token,
                refresh_token: replacement,
                access_token_expires_at,
            }),
            RefreshRotationResult::Rejected => Err(RefreshSessionError::InvalidRefreshToken),
        }
    }
}

/// Failure revoking one owner-scoped session.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RevokeSessionError {
    /// The session is missing or belongs to another user.
    #[error("session not found")]
    SessionNotFound,
    /// The revocation operation failed.
    #[error("session storage failed")]
    Storage(#[source] SessionStoreError),
}
/// Revokes one session belonging to the supplied user.
pub struct RevokeSession<'a, S> {
    store: &'a S,
}
impl<'a, S: SessionStore> RevokeSession<'a, S> {
    /// Wires the session store.
    pub fn new(store: &'a S) -> Self {
        Self { store }
    }
    /// Revokes an owned session; missing and foreign sessions are indistinguishable.
    pub async fn execute(
        &self,
        user_id: UserId,
        session_id: SessionId,
        now: OffsetDateTime,
    ) -> Result<(), RevokeSessionError> {
        if self
            .store
            .revoke_session_for_user(user_id, session_id, now)
            .await
            .map_err(RevokeSessionError::Storage)?
        {
            Ok(())
        } else {
            Err(RevokeSessionError::SessionNotFound)
        }
    }
}

#[cfg(feature = "jwt-hs256")]
mod jwt {
    use super::*;
    use hmac::{Hmac, Mac};
    use serde::{Deserialize, Serialize};
    use std::str::FromStr;

    type HmacSha256 = Hmac<Sha256>;
    const MAX_JWT_LENGTH: usize = 8 * 1024;
    /// A validated HS256 signing key whose debug output is redacted.
    pub struct Hs256SigningSecret(Zeroizing<Vec<u8>>);
    /// An invalid HS256 signing secret.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
    #[non_exhaustive]
    pub enum Hs256SigningSecretError {
        /// The supplied key has fewer than 256 bits.
        #[error("HS256 signing secret must contain at least 32 bytes")]
        TooShort,
    }
    impl Hs256SigningSecret {
        /// Takes ownership of a 256-bit-or-longer signing key.
        pub fn new(bytes: Vec<u8>) -> Result<Self, Hs256SigningSecretError> {
            let bytes = Zeroizing::new(bytes);
            if bytes.len() < 32 {
                Err(Hs256SigningSecretError::TooShort)
            } else {
                Ok(Self(bytes))
            }
        }
    }
    impl fmt::Debug for Hs256SigningSecret {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("Hs256SigningSecret([REDACTED])")
        }
    }
    /// Official HS256-only JWT access-token codec.
    pub struct Hs256JwtAccessTokenCodec {
        secret: Hs256SigningSecret,
        issuer: String,
    }
    impl Hs256JwtAccessTokenCodec {
        /// Creates a codec with an exact expected issuer.
        pub fn new(secret: Hs256SigningSecret, issuer: String) -> Self {
            Self { secret, issuer }
        }
    }
    #[derive(Serialize, Deserialize)]
    struct Header {
        alg: String,
        typ: String,
    }
    #[derive(Serialize, Deserialize)]
    struct Claims {
        sub: String,
        sid: String,
        iat: i64,
        exp: i64,
        iss: String,
    }
    fn encoded<T: Serialize>(value: &T) -> Result<String, AccessTokenError> {
        serde_json::to_vec(value)
            .map(|v| URL_SAFE_NO_PAD.encode(v))
            .map_err(|_| AccessTokenError::SigningFailed)
    }
    impl AccessTokenCodec for Hs256JwtAccessTokenCodec {
        fn issue(&self, claims: &AccessTokenClaims) -> Result<AccessToken, AccessTokenError> {
            let header = encoded(&Header {
                alg: "HS256".into(),
                typ: "JWT".into(),
            })?;
            let payload = encoded(&Claims {
                sub: claims.user_id().to_string(),
                sid: claims.session_id().to_string(),
                iat: claims.issued_at().unix_timestamp(),
                exp: claims.expires_at().unix_timestamp(),
                iss: self.issuer.clone(),
            })?;
            let signed = format!("{header}.{payload}");
            let mut mac = HmacSha256::new_from_slice(&self.secret.0)
                .map_err(|_| AccessTokenError::SigningFailed)?;
            mac.update(signed.as_bytes());
            let token = format!(
                "{signed}.{}",
                URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
            );
            if token.len() > MAX_JWT_LENGTH {
                return Err(AccessTokenError::SigningFailed);
            }
            Ok(AccessToken::new(token))
        }
        fn verify(
            &self,
            token: &AccessToken,
            now: OffsetDateTime,
        ) -> Result<AccessTokenClaims, AccessTokenError> {
            if token.expose_secret().len() > MAX_JWT_LENGTH {
                return Err(AccessTokenError::InvalidToken);
            }
            let mut parts = token.expose_secret().split('.');
            let (Some(h), Some(p), Some(s), None) =
                (parts.next(), parts.next(), parts.next(), parts.next())
            else {
                return Err(AccessTokenError::InvalidToken);
            };
            let header_bytes = Zeroizing::new(
                URL_SAFE_NO_PAD
                    .decode(h)
                    .map_err(|_| AccessTokenError::InvalidToken)?,
            );
            let header: Header = serde_json::from_slice(&header_bytes)
                .map_err(|_| AccessTokenError::InvalidToken)?;
            if header.alg != "HS256" || header.typ != "JWT" {
                return Err(AccessTokenError::InvalidToken);
            }
            let signature = Zeroizing::new(
                URL_SAFE_NO_PAD
                    .decode(s)
                    .map_err(|_| AccessTokenError::InvalidToken)?,
            );
            let mut mac = HmacSha256::new_from_slice(&self.secret.0)
                .map_err(|_| AccessTokenError::VerificationFailed)?;
            mac.update(format!("{h}.{p}").as_bytes());
            mac.verify_slice(&signature)
                .map_err(|_| AccessTokenError::InvalidToken)?;
            let claim_bytes = Zeroizing::new(
                URL_SAFE_NO_PAD
                    .decode(p)
                    .map_err(|_| AccessTokenError::InvalidToken)?,
            );
            let claims: Claims =
                serde_json::from_slice(&claim_bytes).map_err(|_| AccessTokenError::InvalidToken)?;
            if claims.iss != self.issuer {
                return Err(AccessTokenError::InvalidToken);
            }
            let issued_at = OffsetDateTime::from_unix_timestamp(claims.iat)
                .map_err(|_| AccessTokenError::InvalidClaims)?;
            let expires_at = OffsetDateTime::from_unix_timestamp(claims.exp)
                .map_err(|_| AccessTokenError::InvalidClaims)?;
            if expires_at <= issued_at {
                return Err(AccessTokenError::InvalidClaims);
            }
            if now >= expires_at {
                return Err(AccessTokenError::Expired);
            }
            let user_id =
                UserId::from_str(&claims.sub).map_err(|_| AccessTokenError::InvalidClaims)?;
            let session_id =
                SessionId::from_str(&claims.sid).map_err(|_| AccessTokenError::InvalidClaims)?;
            AccessTokenClaims::new(user_id, session_id, issued_at, expires_at)
        }
    }
}
#[cfg(feature = "jwt-hs256")]
pub use jwt::{Hs256JwtAccessTokenCodec, Hs256SigningSecret, Hs256SigningSecretError};

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "jwt-hs256")]
    use std::str::FromStr;

    fn now() -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap()
    }

    #[test]
    fn session_lifecycle_preserves_first_revocation() {
        let created = now();
        let mut session = Session::new(
            SessionId::new(),
            UserId::new(),
            created,
            created + Duration::days(1),
        )
        .unwrap();
        assert!(session.is_active(created));
        assert!(!session.is_active(session.expires_at()));
        session.revoke(created + Duration::seconds(1));
        session.revoke(created + Duration::seconds(2));
        assert_eq!(session.revoked_at(), Some(created + Duration::seconds(1)));
        assert!(!session.is_active(created));
        assert_eq!(
            Session::restore(
                session.id(),
                session.user_id(),
                session.created_at(),
                session.expires_at(),
                session.revoked_at(),
            )
            .unwrap()
            .revoked_at(),
            session.revoked_at()
        );
        assert!(matches!(
            Session::new(SessionId::new(), UserId::new(), created, created),
            Err(SessionError::InvalidExpiration)
        ));
    }

    #[test]
    fn policy_defaults_and_rejections_are_explicit() {
        let policy = SessionPolicy::default();
        assert_eq!(policy.access_token_ttl(), Duration::minutes(15));
        assert_eq!(policy.session_ttl(), Duration::days(30));
        assert!(matches!(
            SessionPolicy::new(Duration::ZERO, Duration::days(1)),
            Err(SessionPolicyError::NonPositiveDuration)
        ));
        assert!(matches!(
            SessionPolicy::new(Duration::milliseconds(999), Duration::seconds(1)),
            Err(SessionPolicyError::AccessTokenTtlTooShort)
        ));
        assert!(SessionPolicy::new(Duration::seconds(1), Duration::seconds(1)).is_ok());
        assert!(matches!(
            SessionPolicy::new(Duration::days(2), Duration::days(1)),
            Err(SessionPolicyError::AccessExceedsSession)
        ));
    }

    #[test]
    fn refresh_tokens_are_canonical_redacted_and_fingerprinted() {
        let provider = OsRefreshTokenProvider;
        let first = provider.generate().unwrap();
        let second = provider.generate().unwrap();
        assert_eq!(first.expose_secret().len(), 43);
        assert!(!first.expose_secret().contains('='));
        assert_ne!(first.expose_secret(), second.expose_secret());
        assert_eq!(provider.hash(&first), provider.hash(&first));
        assert_ne!(provider.hash(&first), provider.hash(&second));
        assert_eq!(provider.hash(&first).as_bytes().len(), 32);
        assert!(!format!("{first:?}").contains(first.expose_secret()));
        assert!(RefreshToken::parse("not-a-token".into()).is_err());
        assert!(RefreshToken::parse("A".repeat(1024 * 1024)).is_err());
        assert!(RefreshToken::parse(URL_SAFE_NO_PAD.encode([0_u8; 31])).is_err());
        assert!(RefreshToken::parse(first.expose_secret().to_owned()).is_ok());
    }

    #[cfg(feature = "jwt-hs256")]
    #[test]
    fn hs256_jwt_round_trips_with_explicit_expiration_boundary() {
        let codec = Hs256JwtAccessTokenCodec::new(
            Hs256SigningSecret::new(vec![7; 32]).unwrap(),
            "windwatcher".into(),
        );
        let user_id = UserId::from_str("018f6f34-0000-7000-8000-000000000000").unwrap();
        let session_id = SessionId::from_str("018f6f35-0000-7000-8000-000000000000").unwrap();
        let claims = AccessTokenClaims::new(
            user_id,
            session_id,
            now() + Duration::milliseconds(500),
            now() + Duration::minutes(1) + Duration::milliseconds(500),
        )
        .unwrap();
        assert_eq!(claims.issued_at(), now());
        assert_eq!(claims.expires_at(), now() + Duration::minutes(1));
        let token = codec.issue(&claims).unwrap();
        assert_eq!(codec.verify(&token, now()).unwrap(), claims);
        assert!(matches!(
            codec.verify(&token, claims.expires_at()),
            Err(AccessTokenError::Expired)
        ));
        assert!(matches!(
            codec.verify(&token, claims.expires_at() + Duration::seconds(1)),
            Err(AccessTokenError::Expired)
        ));
        let wrong_key = Hs256JwtAccessTokenCodec::new(
            Hs256SigningSecret::new(vec![8; 32]).unwrap(),
            "windwatcher".into(),
        );
        assert!(matches!(
            wrong_key.verify(&token, now()),
            Err(AccessTokenError::InvalidToken)
        ));
        let wrong_issuer = Hs256JwtAccessTokenCodec::new(
            Hs256SigningSecret::new(vec![7; 32]).unwrap(),
            "other".into(),
        );
        assert!(matches!(
            wrong_issuer.verify(&token, now()),
            Err(AccessTokenError::InvalidToken)
        ));
        let none_header = URL_SAFE_NO_PAD.encode(r#"{"alg":"none","typ":"JWT"}"#);
        assert!(matches!(
            codec.verify(&AccessToken::new(format!("{none_header}.x.y")), now()),
            Err(AccessTokenError::InvalidToken)
        ));
        assert!(matches!(
            codec.verify(&AccessToken::new("a".repeat(8 * 1024 + 1)), now()),
            Err(AccessTokenError::InvalidToken)
        ));
        let oversized_issuer = Hs256JwtAccessTokenCodec::new(
            Hs256SigningSecret::new(vec![7; 32]).unwrap(),
            "x".repeat(8 * 1024),
        );
        assert!(matches!(
            oversized_issuer.issue(&claims),
            Err(AccessTokenError::SigningFailed)
        ));
        assert!(!format!("{token:?}").contains(token.expose_secret()));
        assert!(Hs256SigningSecret::new(vec![0; 31]).is_err());
    }
}
