use super::{user_role::UserRole, user_status::UserStatus};
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "users")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: Uuid,
    pub username: String,
    pub name: String,
    pub password_hash: String,
    #[sea_orm(default_value = "user")]
    pub role: UserRole,
    #[sea_orm(default_value = "active")]
    pub status: UserStatus,
}

impl ActiveModelBehavior for ActiveModel {}
