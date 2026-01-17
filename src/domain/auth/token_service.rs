use super::{authenticated_user::AuthenticatedUser, token::Token};

#[derive(Debug)]
pub enum AuthError {
    InvalidToken,
}

pub trait TokenService {
    fn generate(&self, user: &AuthenticatedUser) -> Token;
    fn verify(&self, token: &Token) -> Result<AuthenticatedUser, AuthError>;
}
