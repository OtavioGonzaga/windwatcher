use crate::domain::user::error::UserError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Username(String);

impl Username {
    pub fn new(value: String) -> Result<Self, UserError> {
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}
