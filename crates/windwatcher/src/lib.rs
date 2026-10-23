#![warn(missing_docs)]

//! Embeddable messaging engine for Rust applications.

mod authentication;
mod id;
mod identity;
mod identity_store;
mod local_users;
mod password;
mod session;
#[cfg(feature = "sqlite")]
mod storage;

pub use authentication::{AuthenticateLocalUser, AuthenticateLocalUserError};
pub use identity_store::{IdentityStore, IdentityStoreError, LocalAuthentication};
pub use local_users::{
    CreateLocalUser, CreateLocalUserError, DisableUser, EnableUser, ManageUserError,
    RegisterLocalUser, RegisterLocalUserError, RegistrationPolicy,
};
pub use password::{
    Argon2idPasswordHasher, PasswordHash, PasswordHashError, PasswordHasher, PasswordPolicyError,
    SecretPassword,
};
pub use session::{
    AccessToken, AccessTokenClaims, AccessTokenCodec, AccessTokenError, CreateSession,
    CreateSessionError, OsRefreshTokenProvider, RefreshRotationResult, RefreshSession,
    RefreshSessionError, RefreshToken, RefreshTokenError, RefreshTokenHash, RefreshTokenProvider,
    RevokeSession, RevokeSessionError, Session, SessionError, SessionGrant, SessionPolicy,
    SessionPolicyError, SessionStore, SessionStoreError,
};
#[cfg(feature = "jwt-hs256")]
pub use session::{Hs256JwtAccessTokenCodec, Hs256SigningSecret, Hs256SigningSecretError};

pub use id::{ConversationId, IdentifierError, MessageId, SessionId, UserId};
pub use identity::{User, Username, UsernameError};
#[cfg(feature = "sqlite")]
pub use storage::{MigrationStatus, SqliteStorage, SqliteStorageError, SqliteStorageOptions};
