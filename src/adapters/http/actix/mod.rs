mod api_error;
pub mod auth;
pub mod server;
pub mod user;

use utoipa::{
    Modify, OpenApi,
    openapi::{
        OpenApi as OpenApiStruct,
        security::{Flow, OAuth2, Password, Scopes, SecurityScheme},
    },
};

pub struct JwtSecurityAddon;

impl Modify for JwtSecurityAddon {
    fn modify(&self, openapi: &mut OpenApiStruct) {
        let mut components = openapi.components.take().unwrap_or_default();

        components.add_security_scheme(
            "oauth2_password",
            SecurityScheme::OAuth2(OAuth2::new([Flow::Password(Password::new(
                "/auth/login",
                Scopes::new(),
            ))])),
        );

        openapi.components = Some(components);
    }
}

#[derive(OpenApi)]
#[openapi(
    nest((path = "/users", api = user::UserApiDoc),(path = "/auth", api = auth::AuthApiDoc)),
    modifiers(&JwtSecurityAddon),
    security(
        ("oauth2_password" = [])
    ),
    info(
        title = "Windwatcher",
        version = "0.1.0",
        description = "Hexagonal architecture API"
    )
)]
pub struct ApiDoc;
