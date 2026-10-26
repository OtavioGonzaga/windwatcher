use utoipa::openapi::{
    InfoBuilder, OpenApi, Tag,
    security::{HttpAuthScheme, HttpBuilder, SecurityScheme},
};

pub(super) fn finish(mut spec: OpenApi) -> OpenApi {
    spec.info = InfoBuilder::new().title("Windwatcher API").version(env!("CARGO_PKG_VERSION")).description(Some("Windwatcher HTTP API v1. Tokens are JSON bearer secrets; this protocol is not OAuth or OIDC.")).build();
    spec.tags = Some(vec![Tag::new("Users"), Tag::new("Sessions")]);
    spec.components
        .get_or_insert_with(Default::default)
        .add_security_scheme(
            "bearerAuth",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .bearer_format("JWT")
                    .build(),
            ),
        );
    spec
}
