use crate::domain::user::error::UserError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PasswordHash(String);

impl PasswordHash {
    pub fn new(hash: String) -> Result<Self, UserError> {
        Ok(Self(hash))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}
