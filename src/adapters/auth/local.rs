use uuid::Uuid;

use crate::{
    application::{
        auth::{
            authenticated_user::AuthenticatedUser, authenticator::Authenticator,
            credentials::Credentials, error::AuthenticationError,
        },
        security::{error::TokenError, token_service::TokenService},
    },
    domain::{
        errors::repository::RepositoryError,
        user::{entity::User, password_hasher::PasswordHasher, repository::UserRepository},
    },
};

#[derive(Clone)]
pub struct LocalAuthenticator<U, H, T>
where
    U: UserRepository,
    H: PasswordHasher,
    T: TokenService,
{
    user_repository: U,
    hasher: H,
    token_service: T,
}

impl<U, H, T> LocalAuthenticator<U, H, T>
where
    U: UserRepository,
    H: PasswordHasher,
    T: TokenService,
{
    pub fn new(user_repository: U, hasher: H, token_service: T) -> Self {
        Self {
            user_repository,
            hasher,
            token_service,
        }
    }
}

impl<U, H, T> Authenticator for LocalAuthenticator<U, H, T>
where
    U: UserRepository,
    H: PasswordHasher,
    T: TokenService,
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
            Credentials::RefreshToken(refresh_token) => {
                let id: Uuid = self.token_service.verify_refresh(&refresh_token)?;
                let user: User = self
                    .user_repository
                    .find_by_id(&id)
                    .await?
                    .ok_or(AuthenticationError::UserNotFound)?;

                if !user.is_active() {
                    return Err(AuthenticationError::UserInactive);
                }

                Ok(AuthenticatedUser::from(user))
            }
        }
    }
}

impl From<RepositoryError> for AuthenticationError {
    fn from(_: RepositoryError) -> Self {
        AuthenticationError::ProviderUnavailable
    }
}

impl From<TokenError> for AuthenticationError {
    fn from(value: TokenError) -> Self {
        match value {
            TokenError::Internal => AuthenticationError::ProviderUnavailable,
            _ => AuthenticationError::InvalidCredentials,
        }
    }
}
