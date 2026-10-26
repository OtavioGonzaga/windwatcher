use axum::extract::{FromRequestParts, Path, Query};
use axum::http::request::Parts;
use axum::{
    Json,
    extract::{FromRequest, Request, rejection::JsonRejection},
    http::StatusCode,
};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use utoipa::IntoParams;

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

pub(super) struct ApiQuery<T>(pub T);
impl<S: Send + Sync, T: DeserializeOwned> FromRequestParts<S> for ApiQuery<T> {
    type Rejection = ApiProblem;
    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let context = *parts
            .extensions
            .get::<RequestContext>()
            .expect("outer request context");
        Query::<T>::from_request_parts(parts, state)
            .await
            .map(|Query(value)| Self(value))
            .map_err(|_| ApiProblem::new(ProblemKind::InvalidQuery, context))
    }
}

pub(super) struct ApiPath<T>(pub T);
impl<S: Send + Sync, T: DeserializeOwned + Send> FromRequestParts<S> for ApiPath<T> {
    type Rejection = ApiProblem;
    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let context = *parts
            .extensions
            .get::<RequestContext>()
            .expect("outer request context");
        Path::<T>::from_request_parts(parts, state)
            .await
            .map(|Path(value)| Self(value))
            .map_err(|_| ApiProblem::new(ProblemKind::InvalidPath, context))
    }
}

pub(super) fn identifier<T: std::str::FromStr>(
    value: &str,
    context: RequestContext,
) -> Result<T, ApiProblem> {
    value
        .parse()
        .map_err(|_| ApiProblem::new(ProblemKind::InvalidPath, context))
}

#[derive(Default, Deserialize, IntoParams)]
#[serde(deny_unknown_fields)]
#[into_params(parameter_in = Query)]
pub(super) struct PaginationQuery {
    /// Opaque next-only continuation cursor.
    pub cursor: Option<String>,
    /// Page size; defaults to 50, inclusive maximum 100.
    #[param(default = 50, minimum = 1, maximum = 100)]
    pub limit: Option<i64>,
}
impl PaginationQuery {
    pub fn page(self, context: RequestContext) -> Result<windwatcher::PageRequest, ApiProblem> {
        let limit = self.limit.unwrap_or(50);
        let limit = usize::try_from(limit)
            .ok()
            .and_then(|n| windwatcher::PageLimit::new(n).ok())
            .ok_or_else(|| ApiProblem::new(ProblemKind::InvalidLimit, context))?;
        let request = windwatcher::PageRequest::new(limit);
        Ok(match self.cursor {
            Some(token) => request.with_cursor(token),
            None => request,
        })
    }
}

pub(super) struct CommandKey(pub Option<windwatcher::IdempotencyKey>);
impl<S: Send + Sync> FromRequestParts<S> for CommandKey {
    type Rejection = ApiProblem;
    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        let context = *parts
            .extensions
            .get::<RequestContext>()
            .expect("outer request context");
        let invalid = || ApiProblem::new(ProblemKind::InvalidIdempotencyKey, context);
        let mut values = parts.headers.get_all("idempotency-key").iter();
        let key = match values.next() {
            None => None,
            Some(value) => {
                if values.next().is_some() {
                    return Err(invalid());
                }
                Some(
                    windwatcher::IdempotencyKey::new(value.to_str().map_err(|_| invalid())?)
                        .map_err(|_| invalid())?,
                )
            }
        };
        Ok(Self(key))
    }
}
