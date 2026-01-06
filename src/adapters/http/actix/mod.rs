pub mod server;
pub mod user;

use crate::adapters::http::actix::user::{dto, handler};
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    paths(
        handler::create_user,
        handler::find_by_id
    ),
    components(
        schemas(
            dto::CreateUserHttpDto,
            dto::CreateUserResponseDto
        )
    ),
    tags(
        (name = "Users", description = "User management endpoints")
    )
)]
pub struct ApiDoc;
