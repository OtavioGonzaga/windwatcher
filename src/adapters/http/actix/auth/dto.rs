use serde::Deserialize;
use utoipa::ToSchema;

#[derive(Deserialize, ToSchema)]
pub enum GrantType  {
    #[serde(rename = "password")]
    Password,
    #[serde(rename = "refresh_token")]
    RefreshToken,
    #[serde(other)]
    Unsupported,
}

#[derive(Deserialize, ToSchema)]
pub struct TokenRequest {
    /// OAuth2 grant type
    pub grant_type: GrantType,

    /// Username (password grant)
    pub username: Option<String>,

    /// Password (password grant)
    pub password: Option<String>,

    /// Refresh token (refresh_token grant)
    pub refresh_token: Option<String>,
}
