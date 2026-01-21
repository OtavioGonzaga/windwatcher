use crate::{application::{auth::authenticated_user::AuthenticatedUser, security::{error::TokenError, token::{IssuedToken, RefreshToken, Token}, token_service::TokenService}}, domain::user::entity::UserRole};
use jsonwebtoken::{
    DecodingKey, EncodingKey, Header, TokenData, Validation, decode, encode, errors::ErrorKind,
};
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
pub struct JwtService {
    secret: String,
    ttl_seconds: u64,
}

impl JwtService {
    pub fn new(jwt_secret: String, ttl_seconds: u64) -> Self {
        let secret: String = jwt_secret;

        Self {
            secret,
            ttl_seconds,
        }
    }
}

impl TokenService for JwtService {
    fn issue(&self, user: &AuthenticatedUser) -> Result<IssuedToken, TokenError> {
        let expires_in: u64 = self.ttl_seconds;

        let exp: u64 = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| TokenError::Internal)?
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
        )?;
        let refresh_token: Option<RefreshToken> = None; 

        let token: Token = Token::new(jwt);
        let issued_token: IssuedToken = IssuedToken::new(token, expires_in, refresh_token);

        Ok(issued_token)
    }

    fn verify(&self, token: &Token) -> Result<AuthenticatedUser, TokenError> {
        let data: TokenData<Claims> = decode::<Claims>(
            token.as_str(),
            &DecodingKey::from_secret(self.secret.as_bytes()),
            &Validation::default(),
        )?;

        Ok(AuthenticatedUser::new(
            data.claims.sub.parse().unwrap(),
            data.claims.username,
            data.claims.roles,
        ))
    }
}

impl From<jsonwebtoken::errors::Error> for TokenError {
    fn from(value: jsonwebtoken::errors::Error) -> Self {
        match value.kind() {
            ErrorKind::InvalidToken
            | ErrorKind::InvalidAlgorithm
            | ErrorKind::InvalidAlgorithmName
            | ErrorKind::InvalidKeyFormat
            | ErrorKind::MissingAlgorithm
            | ErrorKind::Utf8(_)
            | ErrorKind::Base64(_)
            | ErrorKind::Json(_) => TokenError::Malformed,
            ErrorKind::InvalidSignature
            | ErrorKind::InvalidEcdsaKey
            | ErrorKind::InvalidEddsaKey
            | ErrorKind::InvalidRsaKey(_)
            | ErrorKind::RsaFailedSigning => TokenError::InvalidSignature,
            ErrorKind::ExpiredSignature => TokenError::Expired,
            ErrorKind::ImmatureSignature => TokenError::NotYetValid,
            ErrorKind::InvalidIssuer => TokenError::InvalidIssuer,
            ErrorKind::InvalidAudience => TokenError::InvalidAudience,
            ErrorKind::InvalidSubject => TokenError::Invalid,
            ErrorKind::MissingRequiredClaim(_) => TokenError::Invalid,
            _ => TokenError::Internal,
        }
    }
}
