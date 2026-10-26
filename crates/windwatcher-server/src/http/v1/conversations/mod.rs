pub(super) mod dto;
pub(super) mod handlers;

use crate::http::HttpState;
use utoipa_axum::{router::OpenApiRouter, routes};

pub(super) fn router() -> OpenApiRouter<HttpState> {
    OpenApiRouter::new()
        .routes(routes!(handlers::list, handlers::create))
        .routes(routes!(handlers::get, handlers::update))
        .routes(routes!(handlers::list_members))
        .routes(routes!(
            handlers::add_member,
            handlers::update_member,
            handlers::remove_member
        ))
        .routes(routes!(handlers::transfer_owner))
        .routes(routes!(handlers::leave))
}
