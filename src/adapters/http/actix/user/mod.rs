pub mod dto;
pub mod handler;
pub mod routes;

use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    paths(
        handler::create_user,
        handler::find_by_id,
        handler::delete_user
    ),
    components(
        schemas(
            dto::CreateUserDto,
            dto::UserResponseDto
        )
    ),
    tags(
        (name = "Users", description = "User management endpoints")
    )
)]
pub struct UserApiDoc;
