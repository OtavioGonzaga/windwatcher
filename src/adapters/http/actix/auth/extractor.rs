use crate::{
    adapters::auth::jwt::service::JwtTokenService,
    domain::auth::{
        authenticated_user::AuthenticatedUser, token::Token, token_service::TokenService,
    },
};
use actix_web::{Error, FromRequest, HttpRequest, web};
use std::future::{Ready, ready};

impl FromRequest for AuthenticatedUser {
    type Error = Error;
    type Future = Ready<Result<Self, Error>>;

    fn from_request(req: &HttpRequest, _: &mut actix_web::dev::Payload) -> Self::Future {
        let auth_header: Option<Token> = req
            .headers()
            .get("Authorization")
            .and_then(|h| h.to_str().ok())
            .and_then(|h| h.strip_prefix("Bearer "))
            .map(Token::new);

        let jwt_service: &web::Data<JwtTokenService> = req
            .app_data::<web::Data<JwtTokenService>>()
            .expect("Login service missing");

        match auth_header {
            Some(token) => match jwt_service.verify(&token) {
                Ok(user) => ready(Ok(user)),
                Err(_) => ready(Err(actix_web::error::ErrorUnauthorized("Invalid token"))),
            },
            None => ready(Err(actix_web::error::ErrorUnauthorized("Missing token"))),
        }
    }
}
