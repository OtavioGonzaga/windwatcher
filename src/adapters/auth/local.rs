use crate::{
    application::auth::{
        authenticated_user::AuthenticatedUser, authenticator::Authenticator,
        credentials::Credentials, error::AuthenticationError,
    },
    domain::{
        errors::repository::RepositoryError,
        user::{entity::User, password_hasher::PasswordHasher, repository::UserRepository},
    },
};

#[derive(Clone)]
pub struct LocalAuthenticator<U, H>
where
    U: UserRepository,
    H: PasswordHasher,
{
    user_repository: U,
    hasher: H,
}

impl<U, H> LocalAuthenticator<U, H>
where
    U: UserRepository,
    H: PasswordHasher,
{
    pub fn new(user_repository: U, hasher: H) -> Self {
        Self {
            user_repository,
            hasher,
        }
    }
}

impl<U, H> Authenticator for LocalAuthenticator<U, H>
where
    U: UserRepository,
    H: PasswordHasher,
{
    async fn authenticate(
        &self,
        credentials: Credentials,
    ) -> Result<AuthenticatedUser, AuthenticationError> {
        match credentials {
            Credentials::UsernamePassword { username, password } => {
                let user: User = self
                    .user_repository
                    .find_by_username(&username)
                    .await?
                    .ok_or(AuthenticationError::UserNotFound)?;

                if !user.is_active() {
                    return Err(AuthenticationError::UserInactive);
                }

                if !self
                    .hasher
                    .verify(password.as_str(), user.password_hash.as_str())
                {
                    return Err(AuthenticationError::InvalidCredentials);
                }

                Ok(AuthenticatedUser::from(user))
            }
            _ => Err(AuthenticationError::UnsupportedCredentials),
        }
    }
}

impl From<RepositoryError> for AuthenticationError {
    fn from(_: RepositoryError) -> Self {
        AuthenticationError::ProviderUnavailable
    }
}
