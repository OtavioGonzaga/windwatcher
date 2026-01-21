use super::{error::TokenError, token::Token};
use crate::application::{auth::authenticated_user::AuthenticatedUser, security::token::IssuedToken};

pub trait TokenService {
    fn issue(&self, user: &AuthenticatedUser) -> Result<IssuedToken, TokenError>;
    fn verify(&self, token: &Token) -> Result<AuthenticatedUser, TokenError>;
}
