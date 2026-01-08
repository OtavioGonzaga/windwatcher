pub mod server;
pub mod user;

use crate::adapters::http::actix;
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    paths(
        actix::user::handler::create_user,
        actix::user::handler::find_by_id
    ),
    components(
        schemas(
            actix::user::dto::CreateUserHttpDto,
            actix::user::dto::UserResponseDto
        )
    ),
    tags(
        (name = "Users", description = "User management endpoints")
    )
)]
pub struct ApiDoc;
