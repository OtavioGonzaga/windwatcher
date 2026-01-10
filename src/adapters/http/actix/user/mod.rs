pub mod dto;
pub mod handler;
pub mod routes;

use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    paths(
        handler::find_by_id,
        handler::create_user,
        handler::update_user,
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
