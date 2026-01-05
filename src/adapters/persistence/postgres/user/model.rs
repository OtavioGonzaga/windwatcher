use sea_orm::ActiveValue::Set;

use crate::domain::user::{
    entity::User,
    value_objects::{password_hash::PasswordHash, username::Username},
};

use super::entity::{ActiveModel, Model};

impl TryFrom<Model> for User {
    type Error = crate::domain::user::error::UserError;

    fn try_from(model: Model) -> Result<Self, Self::Error> {
        Ok(User {
            id: model.id,
            username: Username::new(model.username)?,
            name: model.name,
            password_hash: PasswordHash::new(model.password_hash)?,
        })
    }
}

impl From<User> for ActiveModel {
    fn from(user: User) -> Self {
        ActiveModel {
            id: Set(user.id),
            username: Set(user.username.as_str().to_owned()),
            name: Set(user.name),
            password_hash: Set(user.password_hash.as_str().to_owned()),
        }
    }
}
