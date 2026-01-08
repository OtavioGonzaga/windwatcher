use crate::domain::user::error::UserError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PasswordPlain(String);

impl PasswordPlain {
    pub fn new(password: String) -> Result<Self, UserError> {
        Self::validate_password(&password)?;

        Ok(Self(password))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn validate_password(password: &str) -> Result<(), UserError> {
        if password.is_empty() || password.chars().count() < 8 {
            return Err(UserError::InvalidPassword(
                "Password must be at least 8 characters long".into(),
            ));
        }

        if password.chars().count() > 64 {
            return Err(UserError::InvalidPassword(
                "Password must be at most 64 characters long".into(),
            ));
        }

        Ok(())
    }
}
