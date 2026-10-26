use serde::{Deserialize, Serialize};
use time::{OffsetDateTime, UtcOffset};
use utoipa::ToSchema;
use windwatcher::User;
use zeroize::Zeroizing;

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(in crate::http) struct RegisterUserRequest {
    #[schema(example = "alice")]
    pub username: String,
    #[schema(value_type = String, write_only, example = "fake example password")]
    pub password: Zeroizing<String>,
}

#[derive(Serialize, ToSchema)]
pub(in crate::http) struct UserResponse {
    #[schema(example = "019a0000-0000-7000-8000-000000000001")]
    id: String,
    #[schema(example = "alice")]
    username: String,
    #[serde(with = "time::serde::rfc3339::option")]
    #[schema(value_type = Option<String>, format = DateTime)]
    disabled_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    updated_at: OffsetDateTime,
}
impl From<User> for UserResponse {
    fn from(user: User) -> Self {
        Self {
            id: user.id().to_string(),
            username: user.username().as_str().to_owned(),
            disabled_at: user.disabled_at().map(|t| t.to_offset(UtcOffset::UTC)),
            created_at: user.created_at().to_offset(UtcOffset::UTC),
            updated_at: user.updated_at().to_offset(UtcOffset::UTC),
        }
    }
}
