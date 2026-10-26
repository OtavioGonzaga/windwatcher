pub(super) mod dto;
mod handlers;

use crate::http::HttpState;
use utoipa_axum::{router::OpenApiRouter, routes};

pub(super) fn router() -> OpenApiRouter<HttpState> {
    OpenApiRouter::new()
        .routes(routes!(handlers::list, handlers::send))
        .routes(routes!(handlers::get, handlers::edit, handlers::delete))
        .routes(routes!(handlers::read_state))
}
