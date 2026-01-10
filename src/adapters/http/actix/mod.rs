pub mod server;
pub mod user;

use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    nest((path = "/users", api = user::UserApiDoc)),
    info(
        title = "My API",
        version = "1.0.0",
        description = "Hexagonal architecture API"
    )
)]
pub struct ApiDoc;
