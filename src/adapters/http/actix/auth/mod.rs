pub mod dto;
pub mod extractor;
pub mod handler;
pub mod routes;
pub mod middleware;

use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    paths(
        handler::token,
    ),
    components(
        schemas(
            dto::TokenRequest,
        )
    ),
    tags(
        (name = "Auth", description = "Auth endpoints")
    )
)]
pub struct AuthApiDoc;
