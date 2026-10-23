use crate::{
    IdentityStore, IdentityStoreError, PasswordHashError, PasswordHasher, PasswordPolicyError,
    SecretPassword, SessionStore, SessionStoreError, User, UserId, UsernameError,
};
use time::OffsetDateTime;

/// Explicit policy for public self-registration; trusted operator flows are separate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RegistrationPolicy {
    /// Self-registration is permitted.
    Open,
    /// Self-registration is rejected.
    Closed,
}

/// Failures specific to local self-registration.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RegisterLocalUserError {
    /// Self-registration is closed by policy.
    #[error("registration is closed")]
    RegistrationClosed,
    /// Username validation failed.
    #[error("invalid username")]
    InvalidUsername(#[source] UsernameError),
    /// The password does not satisfy new-password policy.
    #[error("invalid new password")]
    InvalidPassword(#[source] PasswordPolicyError),
    /// Atomic creation found a conflicting canonical username.
    #[error("username is already taken")]
    UsernameTaken,
    /// Password hashing failed before persistence.
    #[error("password hashing failed")]
    PasswordHashing(#[source] PasswordHashError),
    /// An operational persistence failure occurred.
    #[error("registration storage failed")]
    Storage(#[source] IdentityStoreError),
}

/// Failures specific to trusted local-user provisioning.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CreateLocalUserError {
    /// Username validation failed.
    #[error("invalid username")]
    InvalidUsername(#[source] UsernameError),
    /// The password does not satisfy new-password policy.
    #[error("invalid new password")]
    InvalidPassword(#[source] PasswordPolicyError),
    /// Atomic creation found a conflicting canonical username.
    #[error("username is already taken")]
    UsernameTaken,
    /// Password hashing failed before persistence.
    #[error("password hashing failed")]
    PasswordHashing(#[source] PasswordHashError),
    /// An operational persistence failure occurred.
    #[error("user creation storage failed")]
    Storage(#[source] IdentityStoreError),
}

async fn create_local_user<S: IdentityStore, H: PasswordHasher>(
    store: &S,
    hasher: &H,
    username: &str,
    password: &SecretPassword,
    now: OffsetDateTime,
) -> Result<User, CreateLocalUserError> {
    let username = username
        .parse()
        .map_err(CreateLocalUserError::InvalidUsername)?;
    password
        .validate_new_password()
        .map_err(CreateLocalUserError::InvalidPassword)?;
    let user = User::new(UserId::new(), username, now);
    let hash = hasher
        .hash(password)
        .await
        .map_err(CreateLocalUserError::PasswordHashing)?;
    store
        .create_local_user(&user, &hash)
        .await
        .map_err(|error| match error {
            IdentityStoreError::UsernameConflict => CreateLocalUserError::UsernameTaken,
            other => CreateLocalUserError::Storage(other),
        })?;
    Ok(user)
}

/// Trusted operator capability to provision a local user; it does not consult
/// self-registration policy or create a session.
pub struct CreateLocalUser<'a, S, H> {
    store: &'a S,
    hasher: &'a H,
}

impl<'a, S: IdentityStore, H: PasswordHasher> CreateLocalUser<'a, S, H> {
    /// Wires persistence and password hashing for trusted provisioning.
    pub fn new(store: &'a S, hasher: &'a H) -> Self {
        Self { store, hasher }
    }

    /// Creates an enabled user at the supplied instant without creating a session.
    pub async fn execute(
        &self,
        username: &str,
        password: &SecretPassword,
        now: OffsetDateTime,
    ) -> Result<User, CreateLocalUserError> {
        create_local_user(self.store, self.hasher, username, password, now).await
    }
}

/// Policy-controlled self-registration using atomic identity persistence.
pub struct RegisterLocalUser<'a, S, H> {
    store: &'a S,
    hasher: &'a H,
    policy: RegistrationPolicy,
}

impl<'a, S: IdentityStore, H: PasswordHasher> RegisterLocalUser<'a, S, H> {
    /// Wires ports and an explicitly chosen policy (there is no implicit default).
    pub fn new(store: &'a S, hasher: &'a H, policy: RegistrationPolicy) -> Self {
        Self {
            store,
            hasher,
            policy,
        }
    }
    /// Creates an enabled user at the supplied instant. Does not create a session.
    pub async fn execute(
        &self,
        username: &str,
        password: &SecretPassword,
        now: OffsetDateTime,
    ) -> Result<User, RegisterLocalUserError> {
        use RegisterLocalUserError::*;
        if self.policy == RegistrationPolicy::Closed {
            return Err(RegistrationClosed);
        }
        create_local_user(self.store, self.hasher, username, password, now)
            .await
            .map_err(|error| match error {
                CreateLocalUserError::InvalidUsername(error) => InvalidUsername(error),
                CreateLocalUserError::InvalidPassword(error) => InvalidPassword(error),
                CreateLocalUserError::UsernameTaken => UsernameTaken,
                CreateLocalUserError::PasswordHashing(error) => PasswordHashing(error),
                CreateLocalUserError::Storage(error) => Storage(error),
            })
    }
}

/// Failures shared by persisted enable/disable operations.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ManageUserError {
    /// The requested user does not exist (including deletion during update).
    #[error("user not found")]
    UserNotFound,
    /// An operational persistence failure occurred.
    #[error("user state storage failed")]
    Storage(#[source] IdentityStoreError),
    /// An operational failure prevented session revocation after disabling.
    #[error("session revocation storage failed")]
    SessionStorage(#[source] SessionStoreError),
}

impl From<IdentityStoreError> for ManageUserError {
    fn from(error: IdentityStoreError) -> Self {
        match error {
            IdentityStoreError::NotFound => Self::UserNotFound,
            other => Self::Storage(other),
        }
    }
}

/// Trusted operator capability to disable a persisted user; performs no authorization.
pub struct DisableUser<'a, I, S> {
    store: &'a I,
    sessions: &'a S,
}
impl<'a, I: IdentityStore, S: SessionStore> DisableUser<'a, I, S> {
    /// Wires the persistence port. Callers must establish operator trust.
    pub fn new(store: &'a I, sessions: &'a S) -> Self {
        Self { store, sessions }
    }
    /// Disables the user at `now`. An already-disabled user skips its identity
    /// write but still retries session revocation.
    pub async fn execute(&self, id: UserId, now: OffsetDateTime) -> Result<User, ManageUserError> {
        let mut user = self
            .store
            .find_user(id)
            .await?
            .ok_or(ManageUserError::UserNotFound)?;
        if !user.is_disabled() {
            user.disable(now);
            self.store.update_user(&user).await?;
        }
        self.sessions
            .revoke_all_for_user(id, now)
            .await
            .map_err(ManageUserError::SessionStorage)?;
        Ok(user)
    }
}

/// Trusted operator capability to enable a persisted user; performs no authorization.
pub struct EnableUser<'a, S> {
    store: &'a S,
}
impl<'a, S: IdentityStore> EnableUser<'a, S> {
    /// Wires the persistence port. Callers must establish operator trust.
    pub fn new(store: &'a S) -> Self {
        Self { store }
    }
    /// Enables the user at `now`; an already enabled user causes no write.
    pub async fn execute(&self, id: UserId, now: OffsetDateTime) -> Result<User, ManageUserError> {
        let mut user = self
            .store
            .find_user(id)
            .await?
            .ok_or(ManageUserError::UserNotFound)?;
        if user.is_disabled() {
            user.enable(now);
            self.store.update_user(&user).await?;
        }
        Ok(user)
    }
}
