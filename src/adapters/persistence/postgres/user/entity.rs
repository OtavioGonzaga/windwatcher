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
}

impl ActiveModelBehavior for ActiveModel {}
