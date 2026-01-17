use serde::Deserialize;
use utoipa::ToSchema;

#[derive(Deserialize, ToSchema)]
pub struct LoginRequest {
    /// The username of the user.
    #[schema(min_length = 3, max_length = 32)]
    pub username: String,
    /// The password of the user.
    #[schema(min_length = 8, max_length = 64)]
    pub password: String,
    /// OAuth2 grant type
    pub grant_type: Option<String>,
}
