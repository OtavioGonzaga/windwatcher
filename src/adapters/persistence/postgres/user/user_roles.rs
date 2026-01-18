use crate::domain::user::entity::UserRole as DomainUserRole;
use sea_orm::{DeriveActiveEnum, EnumIter};

#[derive(EnumIter, DeriveActiveEnum, Clone, Debug, PartialEq, Eq, Default)]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "user_roles")]
pub enum UserRole {
    #[sea_orm(string_value = "administrator")]
    Administrator,
    #[sea_orm(string_value = "user")]
    #[default]
    User,
}

impl From<DomainUserRole> for UserRole {
    fn from(value: DomainUserRole) -> Self {
        match value {
            DomainUserRole::Administrator => UserRole::Administrator,
            DomainUserRole::User => UserRole::User,
        }
    }
}

impl From<UserRole> for DomainUserRole {
    fn from(value: UserRole) -> Self {
        match value {
            UserRole::Administrator => DomainUserRole::Administrator,
            UserRole::User => DomainUserRole::User,
        }
    }
}
