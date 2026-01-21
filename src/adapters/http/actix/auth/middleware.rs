use crate::{
    adapters::{http::actix::api_error::ApiError, token::jwt::JwtService},
    application::{
        auth::authenticated_user::AuthenticatedUser,
        security::{token::Token, token_service::TokenService},
    },
};
use actix_web::{
    Error, HttpMessage,
    dev::{Service, ServiceRequest, ServiceResponse, Transform},
    http::StatusCode,
    web,
};
use std::{
    future::{Ready, ready},
    pin::Pin,
    rc::Rc,
    task::Poll,
};

type LocalFuture = Pin<Box<dyn Future<Output = Result<ServiceResponse, Error>>>>;

pub struct AuthMiddleware;

impl<S> Transform<S, ServiceRequest> for AuthMiddleware
where
    S: Service<ServiceRequest, Response = ServiceResponse, Error = Error> + 'static,
{
    type Response = ServiceResponse;
    type Error = Error;
    type InitError = ();
    type Transform = AuthMiddlewareService<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(AuthMiddlewareService {
            service: Rc::new(service),
        }))
    }
}

pub struct AuthMiddlewareService<S> {
    service: Rc<S>,
}

impl<S> Service<ServiceRequest> for AuthMiddlewareService<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse, Error = Error> + 'static,
{
    type Response = ServiceResponse;
    type Error = Error;
    type Future = LocalFuture;

    fn poll_ready(&self, ctx: &mut core::task::Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(ctx)
    }

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let service = self.service.clone();

        Box::pin(async move {
            let token: Option<Token> = req
                .headers()
                .get("Authorization")
                .and_then(|h| h.to_str().ok())
                .and_then(|h| h.strip_prefix("Bearer "))
                .map(Token::new);

            let jwt_service: &web::Data<JwtService> = req
                .app_data::<web::Data<JwtService>>()
                .expect("JwtService missing");

            match token {
                Some(token) => match jwt_service.verify(&token) {
                    Ok(user) => {
                        req.extensions_mut().insert::<AuthenticatedUser>(user);

                        service.call(req).await
                    }
                    Err(_) => Ok(req.into_response(actix_web::HttpResponse::from_error(
                        ApiError::new(StatusCode::UNAUTHORIZED, "Invalid token"),
                    ))),
                },
                None => Ok(req.into_response(actix_web::HttpResponse::from_error(
                        ApiError::new(StatusCode::UNAUTHORIZED, "Missing token"),
                    ))),
            }
        })
    }
}
