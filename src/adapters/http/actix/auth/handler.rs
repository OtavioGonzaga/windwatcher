use super::dto::TokenRequest;
use crate::{
    adapters::{
        auth::local::LocalAuthenticator,
        hash::argon2::Argon2Hasher,
        http::actix::{api_error::ApiError, auth::dto::GrantType},
        persistence::postgres::user::repository::PostgresUserRepository,
        token::jwt::JwtService,
    },
    application::{
        auth::{
            credentials::Credentials,
            error::AuthenticationError,
            login::{Login, LoginError},
        },
        security::token::{IssuedToken, RefreshToken},
    },
    domain::user::value_objects::{password_plain::PasswordPlain, username::Username},
};
use actix_web::{HttpResponse, http::StatusCode, web};
use serde_json::json;

#[utoipa::path(
    post,
    path = "/token",
    tag = "Auth",
    request_body(
        content = TokenRequest,
        content_type = "application/x-www-form-urlencoded"
    ),
    responses(
        (status = 200, description = "User autheticated"),
        (status = 400, description = "Invalid data provided"),
        (status = 401, description = "Invalid credentials")
    )
)]
pub async fn token(
    body: web::Form<TokenRequest>,
    login: web::Data<
        Login<LocalAuthenticator<PostgresUserRepository, Argon2Hasher, JwtService>, JwtService>,
    >,
) -> Result<HttpResponse, ApiError> {
    let credentials: Credentials =
        match body.grant_type {
            GrantType::Password => {
                let username: &String = body.username.as_ref().ok_or_else(|| {
                    ApiError::new(StatusCode::BAD_REQUEST, "username is required")
                })?;

                let password: &String = body.password.as_ref().ok_or_else(|| {
                    ApiError::new(StatusCode::BAD_REQUEST, "password is required")
                })?;

                Credentials::UsernamePassword {
                    username: Username::new(username.clone())?,
                    password: PasswordPlain::new(password.clone())?,
                }
            }

            GrantType::RefreshToken => {
                let refresh_token: &String = body.refresh_token.as_ref().ok_or_else(|| {
                    ApiError::new(StatusCode::BAD_REQUEST, "refresh_token is required")
                })?;

                Credentials::RefreshToken(RefreshToken::new(refresh_token.clone()))
            }

            GrantType::Unsupported => {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "Unsupported grant type",
                ));
            }
        };

    let IssuedToken {
        expires_in,
        token,
        refresh_token,
    } = login.execute(credentials).await?;

    Ok(HttpResponse::Ok().json(json!({
        "access_token": token.as_str(),
        "token_type": "Bearer",
        "expires_in": expires_in,
        "refresh_token": refresh_token.as_ref().map(|t| t.as_str())
    })))
}

impl From<LoginError> for ApiError {
    fn from(value: LoginError) -> Self {
        match value {
            LoginError::Authentication(error) => match error {
                AuthenticationError::_UnsupportedCredentials => {
                    ApiError::new(StatusCode::BAD_REQUEST, "Unsupported credentials type")
                }
                AuthenticationError::UserInactive => {
                    ApiError::new(StatusCode::BAD_REQUEST, "User inactive")
                }
                AuthenticationError::InvalidCredentials => {
                    ApiError::new(StatusCode::UNAUTHORIZED, "Invalid credentials")
                }
                AuthenticationError::UserNotFound => {
                    ApiError::new(StatusCode::NOT_FOUND, "User not found")
                }
                AuthenticationError::ProviderUnavailable => ApiError::internal_server_error(),
            },
            LoginError::Token(_token_error) => ApiError::internal_server_error(),
        }
    }
}
