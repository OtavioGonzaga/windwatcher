pub(in crate::http) mod dto;
mod handlers;

use crate::http::HttpState;
use utoipa_axum::{router::OpenApiRouter, routes};

pub(super) fn router() -> OpenApiRouter<HttpState> {
    OpenApiRouter::new()
        .routes(routes!(handlers::create_session))
        .routes(routes!(handlers::refresh_session))
        .routes(routes!(handlers::revoke_current_session))
}
