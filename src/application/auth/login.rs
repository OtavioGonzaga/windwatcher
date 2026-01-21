use crate::application::{
    auth::{
        authenticated_user::AuthenticatedUser, authenticator::Authenticator,
        credentials::Credentials, error::AuthenticationError,
    },
    security::{error::TokenError, token::IssuedToken, token_service::TokenService},
};

#[derive(Clone)]
pub struct Login<A, T>
where
    A: Authenticator,
    T: TokenService,
{
    authenticator: A,
    token_service: T,
}

impl<A, T> Login<A, T>
where
    A: Authenticator,
    T: TokenService,
{
    pub fn new(authenticator: A, token_service: T) -> Self {
        Self {
            authenticator,
            token_service,
        }
    }

    pub async fn execute(&self, credentials: Credentials) -> Result<IssuedToken, LoginError> {
        let user: AuthenticatedUser = self.authenticator.authenticate(credentials).await?;
        let token: IssuedToken = self.token_service.issue(&user)?;

        Ok(token)
    }
}

pub enum LoginError {
    Authentication(AuthenticationError),
    Token(TokenError),
}

impl From<AuthenticationError> for LoginError {
    fn from(value: AuthenticationError) -> Self {
        LoginError::Authentication(value)
    }
}

impl From<TokenError> for LoginError {
    fn from(value: TokenError) -> Self {
        LoginError::Token(value)
    }
}
