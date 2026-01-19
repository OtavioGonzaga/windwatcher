use crate::domain::{
    auth::{
        authenticated_user::AuthenticatedUser,
        token::Token,
        token_service::{AuthError, TokenService},
    },
    user::entity::UserRole,
};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, TokenData, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    sub: String,
    username: String,
    roles: Vec<UserRole>,
    exp: usize,
}

#[derive(Clone)]
pub struct JwtTokenService {
    secret: String,
    ttl_seconds: u64,
}

impl JwtTokenService {
    pub fn new(jwt_secret: String, ttl_seconds: u64) -> Self {
        let secret: String = jwt_secret;

        Self {
            secret,
            ttl_seconds,
        }
    }
}

impl TokenService for JwtTokenService {
    fn generate(&self, user: &AuthenticatedUser) -> Token {
        let exp: u64 = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + self.ttl_seconds;

        let claims: Claims = Claims {
            sub: user.id.to_string(),
            username: user.username.clone(),
            roles: user.roles.clone(),
            exp: exp as usize,
        };

        let jwt: String = encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(self.secret.as_bytes()),
        )
        .expect("JWT encode failed");

        Token::new(jwt)
    }

    fn verify(&self, token: &Token) -> Result<AuthenticatedUser, AuthError> {
        let data: TokenData<Claims> = decode::<Claims>(
            token.as_str(),
            &DecodingKey::from_secret(self.secret.as_bytes()),
            &Validation::default(),
        )
        .map_err(|_| AuthError::InvalidToken)?;

        Ok(AuthenticatedUser::new(
            data.claims.sub.parse().unwrap(),
            data.claims.username,
            data.claims.roles,
        ))
    }
}
