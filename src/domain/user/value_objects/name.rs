use crate::domain::user::error::UserError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Name(String);

impl Name {
    pub fn new(value: String) -> Result<Self, UserError> {
        Self::validate_name(&value)?;

        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn validate_name(username: &str) -> Result<(), UserError> {
        if username.is_empty() || username.chars().count() < 3 || username.chars().count() > 255 {
            return Err(UserError::InvalidUsername(
                "Name must be between 3 and 255 characters long".into(),
            ));
        }

        Ok(())
    }
}
