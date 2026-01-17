pub mod dto;
pub mod extractor;
pub mod handler;
pub mod routes;

use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    paths(
        handler::login,
    ),
    components(
        schemas(
            dto::LoginRequest,
        )
    ),
    tags(
        (name = "Users", description = "User management endpoints")
    )
)]
pub struct AuthApiDoc;
