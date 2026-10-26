mod sessions;
mod users;

use super::HttpState;
use utoipa_axum::router::OpenApiRouter;

pub(super) fn router() -> OpenApiRouter<HttpState> {
    OpenApiRouter::new()
        .merge(users::router())
        .merge(sessions::router())
}
