mod dto;
mod handlers;
use crate::http::HttpState;
use utoipa_axum::{router::OpenApiRouter, routes};
pub(super) fn router() -> OpenApiRouter<HttpState> {
    OpenApiRouter::new()
        .routes(routes!(handlers::ticket))
        .routes(routes!(handlers::connect))
}
