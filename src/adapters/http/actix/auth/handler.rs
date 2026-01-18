use super::dto::LoginRequest;
use crate::{
    adapters::{
        auth::jwt::service::JwtTokenService, hash::argon2::Argon2Hasher,
        http::actix::api_error::ApiError,
        persistence::postgres::user::repository::PostgresUserRepository,
    },
    application::auth::login::Login,
    domain::{
        auth::token::Token,
        user::value_objects::{password_plain::PasswordPlain, username::Username},
    },
};
use actix_web::{
    HttpResponse,
    http::StatusCode,
    web,
};
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
    login: web::Data<Login<PostgresUserRepository, Argon2Hasher, JwtTokenService>>,
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

    let token: Token = login
        .execute(username, password)
        .await
        .map_err(|_| ApiError::new(StatusCode::UNAUTHORIZED, "Invalid credentials"))?;

    Ok(HttpResponse::Ok().json(json!({
        "access_token": token.as_str(),
        "token_type": "bearer",
        "expires_in": 1800
    })))
}
