use super::dto::LoginRequest;
use crate::{
    adapters::{
        auth::local::LocalAuthenticator, hash::argon2::Argon2Hasher,
        http::actix::api_error::ApiError,
        persistence::postgres::user::repository::PostgresUserRepository, token::jwt::JwtService,
    },
    application::{
        auth::{
            credentials::Credentials,
            error::AuthenticationError,
            login::{Login, LoginError},
        },
        security::token::IssuedToken,
    },
    domain::user::value_objects::{password_plain::PasswordPlain, username::Username},
};
use actix_web::{HttpResponse, http::StatusCode, web};
use serde_json::json;

#[utoipa::path(
    post,
    path = "/login",
    tag = "Auth",
	request_body = LoginRequest,
    responses(
        (status = 200, description = "User autheticated"),
        (status = 400, description = "Invalid data provided"),
        (status = 401, description = "Invalid credentials")
    )
)]
pub async fn login(
    body: web::Form<LoginRequest>,
    login: web::Data<Login<LocalAuthenticator<PostgresUserRepository, Argon2Hasher>, JwtService>>,
) -> Result<HttpResponse, ApiError> {
    if let Some(grant_type) = &body.grant_type {
        if grant_type != "password" {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "Unsupported grant type",
            ));
        }
    }

    let username: Username = Username::new(body.username.clone())?;
    let password: PasswordPlain = PasswordPlain::new(body.password.clone())?;
    let credentials: Credentials = Credentials::UsernamePassword { username, password };

    let IssuedToken {
        expires_in,
        token,
        refresh_token,
    } = login.execute(credentials).await?;

    let refresh_token: Option<&str> = if let Some(refresh_token) = &refresh_token {
        Some(refresh_token.as_str())
    } else {
        None
    };

    Ok(HttpResponse::Ok().json(json!({
        "access_token": token.as_str(),
        "token_type": "Bearer",
        "expires_in": expires_in,
        "refresh_token": refresh_token
    })))
}

impl From<LoginError> for ApiError {
    fn from(value: LoginError) -> Self {
        match value {
            LoginError::Authentication(error) => match error {
                AuthenticationError::UnsupportedCredentials => {
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
            LoginError::Token(_) => ApiError::internal_server_error(),
        }
    }
}
