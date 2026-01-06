use serde::{Serialize, Deserialize};
use utoipa::ToSchema;

use crate::domain::user::entity::User;

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
pub struct CreateUserResponseDto {
    /// The unique identifier of the user.
    pub id: String,
    /// The username of the user.
    pub username: String,
    /// The name of the user.
    pub name: String,
}

impl CreateUserResponseDto {
    pub fn from_domain(user: User) -> Self {
        CreateUserResponseDto {
            id: user.id.to_string(),
            username: user.username.as_str().to_string(),
            name: user.name,
        }
    } 
}