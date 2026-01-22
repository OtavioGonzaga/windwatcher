use uuid::Uuid;

use super::{error::TokenError, token::Token};
use crate::application::{
    auth::authenticated_user::AuthenticatedUser,
    security::token::{IssuedToken, RefreshToken},
};

pub trait TokenService {
    fn issue(&self, user: &AuthenticatedUser) -> Result<IssuedToken, TokenError>;
    fn verify(&self, token: &Token) -> Result<AuthenticatedUser, TokenError>;
    fn verify_refresh(&self, refresh_token: &RefreshToken) -> Result<Uuid, TokenError>;
}
