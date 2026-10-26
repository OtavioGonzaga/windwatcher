use axum::{
    Json,
    extract::{FromRequest, Request, rejection::JsonRejection},
    http::StatusCode,
};
use serde::de::DeserializeOwned;

use super::{
    middleware::request_id::RequestContext,
    problem::{ApiProblem, ProblemKind},
};

/// Shape validation only; semantic validation belongs to application use cases.
pub(super) struct ApiJson<T>(pub T);

impl<S: Send + Sync, T: DeserializeOwned> FromRequest<S> for ApiJson<T> {
    type Rejection = ApiProblem;
    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let context = *request
            .extensions()
            .get::<RequestContext>()
            .expect("outer request context");
        Json::<T>::from_request(request, state)
            .await
            .map(|Json(value)| Self(value))
            .map_err(|error| {
                let kind = match error {
                    JsonRejection::MissingJsonContentType(_) => ProblemKind::UnsupportedMediaType,
                    _ if error.status() == StatusCode::PAYLOAD_TOO_LARGE => {
                        ProblemKind::PayloadTooLarge
                    }
                    _ => ProblemKind::InvalidJson,
                };
                ApiProblem::new(kind, context)
            })
    }
}
