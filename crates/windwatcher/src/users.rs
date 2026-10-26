use crate::{IdentityStore, IdentityStoreError, User, UserId};

/// Failures retrieving a user identity, independently of authentication.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum GetUserError {
    /// No identity exists for the supplied identifier.
    #[error("user not found")]
    UserNotFound,
    /// Identity persistence failed operationally.
    #[error("user lookup storage failed")]
    Storage(#[source] IdentityStoreError),
}

/// Retrieves an identity by ID. Callers are responsible for authorization.
pub struct GetUser<'a, S> {
    store: &'a S,
}

impl<'a, S: IdentityStore> GetUser<'a, S> {
    /// Wires the identity persistence port.
    pub fn new(store: &'a S) -> Self {
        Self { store }
    }

    /// Retrieves an enabled or disabled identity without exposing credentials.
    pub async fn execute(&self, id: UserId) -> Result<User, GetUserError> {
        self.store
            .find_user(id)
            .await
            .map_err(GetUserError::Storage)?
            .ok_or(GetUserError::UserNotFound)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LocalAuthentication, PasswordHash, Username};

    struct Store(Result<Option<User>, ()>);
    impl IdentityStore for Store {
        async fn find_user(&self, id: UserId) -> Result<Option<User>, IdentityStoreError> {
            self.0
                .as_ref()
                .map(|user| user.clone().filter(|u| u.id() == id))
                .map_err(|()| {
                    IdentityStoreError::Unavailable(Box::new(std::io::Error::other("offline")))
                })
        }
        async fn create_local_user(
            &self,
            _: &User,
            _: &PasswordHash,
        ) -> Result<(), IdentityStoreError> {
            unreachable!()
        }
        async fn find_local_auth_by_username(
            &self,
            _: &Username,
        ) -> Result<Option<LocalAuthentication>, IdentityStoreError> {
            unreachable!()
        }
        async fn update_user(&self, _: &User) -> Result<(), IdentityStoreError> {
            unreachable!()
        }
    }

    #[test]
    fn retrieves_existing_identity_including_disabled_state() {
        let mut user = User::new(
            UserId::new(),
            "alice".parse().unwrap(),
            time::OffsetDateTime::UNIX_EPOCH,
        );
        user.disable(time::OffsetDateTime::UNIX_EPOCH);
        let store = Store(Ok(Some(user.clone())));
        assert_eq!(
            futures_executor::block_on(GetUser::new(&store).execute(user.id())).unwrap(),
            user
        );
    }

    #[test]
    fn distinguishes_missing_identity_from_storage_failure() {
        assert!(matches!(
            futures_executor::block_on(GetUser::new(&Store(Ok(None))).execute(UserId::new())),
            Err(GetUserError::UserNotFound)
        ));
        assert!(matches!(
            futures_executor::block_on(GetUser::new(&Store(Err(()))).execute(UserId::new())),
            Err(GetUserError::Storage(IdentityStoreError::Unavailable(_)))
        ));
    }
}
