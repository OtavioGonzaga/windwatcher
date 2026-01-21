use crate::application::auth::authenticated_user::AuthenticatedUser;
use actix_web::{Error, FromRequest, HttpMessage, HttpRequest};
use std::future::{Ready, ready};

impl FromRequest for AuthenticatedUser {
    type Error = Error;
    type Future = Ready<Result<Self, Error>>;

    fn from_request(req: &HttpRequest, _: &mut actix_web::dev::Payload) -> Self::Future {
        match req.extensions().get::<AuthenticatedUser>() {
            Some(user) => ready(Ok(user.clone())),
            None => ready(Err(actix_web::error::ErrorUnauthorized(
                "Not authenticated",
            ))),
        }
    }
}
