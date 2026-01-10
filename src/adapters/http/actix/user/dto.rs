use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Deserialize, ToSchema)]
pub struct CreateUserDto {
    /// The username of the user.
    #[schema(min_length = 3, max_length = 32)]
    pub username: String,
    /// The name of the user.
    #[schema(max_length = 255)]
    pub name: String,
    /// The password of the user.
    #[schema(min_length = 8, max_length = 64)]
    pub password: String,
}

#[derive(Deserialize, ToSchema)]
pub struct UpdateUserDto {
    /// The username of the user.
    #[schema(min_length = 3, max_length = 32)]
    pub username: Option<String>,
    /// The name of the user.
    #[schema( max_length = 255)]
    pub name: Option<String>,
    /// The password of the user.
    #[schema(min_length = 8, max_length = 64)]
    pub password: Option<String>,
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
