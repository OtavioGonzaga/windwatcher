use crate::domain::{
    auth::{authenticated_user::AuthenticatedUser, token::Token, token_service::TokenService},
    errors::repository::RepositoryError,
    user::{
        entity::User,
        password_hasher::PasswordHasher,
        repository::UserRepository,
        value_objects::{password_plain::PasswordPlain, username::Username},
    },
};

#[derive(Clone)]
pub struct Login<U, H, T>
where
    U: UserRepository,
    H: PasswordHasher,
    T: TokenService,
{
    user_repository: U,
    hasher: H,
    token_service: T,
}

impl<U, H, T> Login<U, H, T>
where
    U: UserRepository,
    H: PasswordHasher,
    T: TokenService,
{
    pub fn new(users: U, hasher: H, tokens: T) -> Self {
        Self {
            hasher,
            token_service: tokens,
            user_repository: users,
        }
    }

    pub async fn execute(
        &self,
        username: Username,
        password: PasswordPlain,
    ) -> Result<Token, LoginError> {
        let user: User = self
            .user_repository
            .find_by_username(&username)
            .await?
            .ok_or(LoginError::InvalidCredentials)?;

        if !user.is_active() {
            return Err(LoginError::UserInactive);
        }

        let plain: &str = password.as_str();
        let hash: &str = user.password_hash.as_str();

        let valid: bool = self.hasher.verify(plain, hash);

        if !valid {
            return Err(LoginError::InvalidCredentials);
        }

        let auth_user: AuthenticatedUser = AuthenticatedUser::from(user);

        Ok(self.token_service.generate(&auth_user))
    }
}

pub enum LoginError {
    InvalidCredentials,
    RepositoryError,
    UserInactive,
}

impl From<RepositoryError> for LoginError {
    fn from(_: RepositoryError) -> Self {
        LoginError::RepositoryError
    }
}
