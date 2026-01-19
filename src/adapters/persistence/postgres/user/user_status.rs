use crate::domain::user::entity::UserStatus as DomainUserStatus;
use sea_orm::{DeriveActiveEnum, EnumIter};

#[derive(EnumIter, DeriveActiveEnum, Clone, Debug, PartialEq, Eq, Default)]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "user_status")]
pub enum UserStatus {
    #[sea_orm(string_value = "active")]
    #[default]
    Active,
    #[sea_orm(string_value = "inactive")]
    Inactive,
    #[sea_orm(string_value = "banned")]
    Banned,
}

impl From<DomainUserStatus> for UserStatus {
    fn from(value: DomainUserStatus) -> Self {
        match value {
            DomainUserStatus::Active => UserStatus::Active,
            DomainUserStatus::Inactive => UserStatus::Inactive,
            DomainUserStatus::Banned => UserStatus::Banned,
        }
    }
}

impl From<UserStatus> for DomainUserStatus {
    fn from(value: UserStatus) -> Self {
        match value {
            UserStatus::Active => DomainUserStatus::Active,
            UserStatus::Inactive => DomainUserStatus::Inactive,
            UserStatus::Banned => DomainUserStatus::Banned,
        }
    }
}
