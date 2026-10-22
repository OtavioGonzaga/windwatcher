use crate::{PasswordHash, User, UserId, Username};
use std::{error::Error, future::Future};

/// State needed exclusively for local password authentication.
#[derive(Clone, Debug)]
pub struct LocalAuthentication {
    user: User,
    password_hash: PasswordHash,
}

impl LocalAuthentication {
    /// Assembles a record loaded by a storage adapter.
    pub fn new(user: User, password_hash: PasswordHash) -> Self {
        Self {
            user,
            password_hash,
        }
    }
    /// Returns the identity associated with this credential.
    pub fn user(&self) -> &User {
        &self.user
    }
    /// Returns the credential for verification only.
    pub fn password_hash(&self) -> &PasswordHash {
        &self.password_hash
    }
    /// Extracts the identity without exposing its credential.
    pub fn into_user(self) -> User {
        self.user
    }
}

/// Normalized failures at the identity persistence boundary.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum IdentityStoreError {
    /// Atomic uniqueness enforcement rejected the canonical username.
    #[error("username conflict")]
    UsernameConflict,
    /// The target of an update no longer exists.
    #[error("user not found")]
    NotFound,
    /// Storage is temporarily unavailable.
    #[error("identity storage unavailable")]
    Unavailable(#[source] Box<dyn Error + Send + Sync>),
    /// Another operational storage failure occurred.
    #[error("unexpected identity storage failure")]
    Unexpected(#[source] Box<dyn Error + Send + Sync>),
}

/// Application-driven, asynchronous identity persistence operations.
///
/// Implementations must preserve canonical username uniqueness and keep user
/// identities separate from local credentials. No concrete backend is required.
pub trait IdentityStore: Sync {
    /// Atomically persists both user and credential, or neither. Canonical
    /// username uniqueness MUST hold even for concurrent registration attempts;
    /// a preflight lookup is not sufficient. Return `UsernameConflict` on clash.
    fn create_local_user(
        &self,
        user: &User,
        password_hash: &PasswordHash,
    ) -> impl Future<Output = Result<(), IdentityStoreError>> + Send;
    /// Loads the user and local credential in one logical operation. Returns
    /// `None` when the username or its local credential does not exist.
    fn find_local_auth_by_username(
        &self,
        username: &Username,
    ) -> impl Future<Output = Result<Option<LocalAuthentication>, IdentityStoreError>> + Send;
    /// Loads a user independently of any credentials.
    fn find_user(
        &self,
        id: UserId,
    ) -> impl Future<Output = Result<Option<User>, IdentityStoreError>> + Send;
    /// Persists only enabled/disabled state and its update timestamp for an
    /// existing ID. Must not rename, insert, or modify credentials/creation time.
    /// Returns `NotFound` if the user disappeared. Concurrent updates are
    /// last-write-wins; this contract does not provide optimistic locking.
    fn update_user(
        &self,
        user: &User,
    ) -> impl Future<Output = Result<(), IdentityStoreError>> + Send;
}
