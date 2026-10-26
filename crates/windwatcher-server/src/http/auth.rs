use super::{
    HttpState,
    middleware::request_id::RequestContext,
    problem::{ApiProblem, ProblemKind},
};
use axum::{
    extract::FromRequestParts,
    http::{header, request::Parts},
};
use time::OffsetDateTime;
use windwatcher::{AccessToken, AccessTokenCodec, AccessTokenError, SessionId, UserId};

pub(super) struct AuthenticatedActor {
    pub user_id: UserId,
    pub session_id: SessionId,
}

impl FromRequestParts<HttpState> for AuthenticatedActor {
    type Rejection = ApiProblem;
    async fn from_request_parts(
        parts: &mut Parts,
        state: &HttpState,
    ) -> Result<Self, Self::Rejection> {
        let context = RequestContext::from_parts(parts);
        let invalid = || ApiProblem::new(ProblemKind::InvalidAccessToken, context);
        let mut headers = parts.headers.get_all(header::AUTHORIZATION).iter();
        let value = headers.next().ok_or_else(invalid)?;
        if headers.next().is_some() {
            return Err(invalid());
        }
        let value = value.to_str().map_err(|_| invalid())?;
        let mut fields = value.split_ascii_whitespace();
        let (Some(scheme), Some(token), None) = (fields.next(), fields.next(), fields.next())
        else {
            return Err(invalid());
        };
        if !scheme.eq_ignore_ascii_case("Bearer") {
            return Err(invalid());
        }
        match state.codec.verify(
            &AccessToken::new(token.to_owned()),
            OffsetDateTime::now_utc(),
        ) {
            Ok(claims) => Ok(Self {
                user_id: claims.user_id(),
                session_id: claims.session_id(),
            }),
            Err(
                AccessTokenError::InvalidToken
                | AccessTokenError::Expired
                | AccessTokenError::InvalidClaims,
            ) => Err(invalid()),
            Err(error) => Err(ApiProblem::internal(context, &error)),
        }
    }
}
