pub(in crate::http) mod dto;
mod handlers;

use crate::http::HttpState;
use utoipa_axum::{router::OpenApiRouter, routes};

pub(super) fn router() -> OpenApiRouter<HttpState> {
    OpenApiRouter::new()
        .routes(routes!(handlers::register_user))
        .routes(routes!(handlers::get_current_user))
        .routes(routes!(handlers::find_users))
}
