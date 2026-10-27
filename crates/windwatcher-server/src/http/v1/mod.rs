mod conversations;
mod messages;
mod realtime;
mod sessions;
mod users;

use super::HttpState;
use utoipa_axum::router::OpenApiRouter;

pub(super) fn router() -> OpenApiRouter<HttpState> {
    OpenApiRouter::new()
        .merge(users::router())
        .merge(sessions::router())
        .merge(conversations::router())
        .merge(messages::router())
        .merge(realtime::router())
}
