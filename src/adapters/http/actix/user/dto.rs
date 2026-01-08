use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Deserialize, ToSchema)]
pub struct CreateUserHttpDto {
    /// The username of the user.
    pub username: String,
    /// The name of the user.
    pub name: String,
    /// The password of the user.
    pub password: String,
}

#[derive(Serialize, ToSchema)]
pub struct UserResponseDto {
    /// The unique identifier of the user.
    pub id: String,
    /// The username of the user.
    pub username: String,
    /// The name of the user.
    pub name: String,
}
