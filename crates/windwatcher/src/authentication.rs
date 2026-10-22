use crate::{IdentityStore, PasswordHasher, SecretPassword, User, Username};

/// Local authentication failures, independent of transports and adapter details.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AuthenticateLocalUserError {
    /// Username syntax, missing local credential, or password mismatch.
    #[error("invalid credentials")]
    InvalidCredentials,
    /// Correct credentials belong to a disabled user.
    #[error("user disabled")]
    UserDisabled,
    /// An operational failure prevented authentication. The source is diagnostic.
    #[error("authentication failed internally")]
    Internal(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Verifies local credentials and returns a user, without establishing a session.
pub struct AuthenticateLocalUser<'a, S, H> {
    store: &'a S,
    hasher: &'a H,
}

impl<'a, S: IdentityStore, H: PasswordHasher> AuthenticateLocalUser<'a, S, H> {
    /// Wires the persistence and password ports.
    pub fn new(store: &'a S, hasher: &'a H) -> Self {
        Self { store, hasher }
    }

    /// Authenticates a bounded password without applying new-password policy.
    /// Invalid/missing usernames perform dummy verification. Disabled state is
    /// disclosed only after successful verification of a real credential.
    pub async fn execute(
        &self,
        username: &str,
        password: &SecretPassword,
    ) -> Result<User, AuthenticateLocalUserError> {
        use AuthenticateLocalUserError::*;
        let record = match username.parse::<Username>() {
            Ok(username) => self
                .store
                .find_local_auth_by_username(&username)
                .await
                .map_err(|e| Internal(Box::new(e)))?,
            Err(_) => None,
        };
        let verified = self
            .hasher
            .verify(password, record.as_ref().map(|r| r.password_hash()))
            .await
            .map_err(|e| Internal(Box::new(e)))?;
        let record = record.filter(|_| verified).ok_or(InvalidCredentials)?;
        if record.user().is_disabled() {
            return Err(UserDisabled);
        }
        Ok(record.into_user())
    }
}
