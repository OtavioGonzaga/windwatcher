use axum::{
    extract::{FromRequestParts, Request},
    http::{HeaderValue, request::Parts},
    middleware::Next,
    response::Response,
};
use uuid::Uuid;

/// Authoritative server-generated correlation context, installed before all layers.
#[derive(Clone, Copy)]
pub(crate) struct RequestContext {
    pub request_id: Uuid,
}

impl RequestContext {
    pub fn from_parts(parts: &Parts) -> Self {
        *parts
            .extensions
            .get::<Self>()
            .expect("request context installed by outer router")
    }
}

impl<S: Send + Sync> FromRequestParts<S> for RequestContext {
    type Rejection = std::convert::Infallible;
    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        Ok(Self::from_parts(parts))
    }
}

pub async fn assign(mut request: Request, next: Next) -> Response {
    let context = RequestContext {
        request_id: Uuid::now_v7(),
    };
    let value =
        HeaderValue::from_str(&context.request_id.to_string()).expect("UUID is a header value");
    request.headers_mut().insert("x-request-id", value.clone());
    request.extensions_mut().insert(context);
    let mut response = next.run(request).await;
    response.headers_mut().insert("x-request-id", value);
    response
}
