use std::{error::Error, fmt, str::FromStr};

/// A canonical, validated username for a Windwatcher user.
///
/// Usernames contain from 3 through 32 ASCII characters. The accepted
/// alphabet is lowercase letters, digits, and underscores (`[a-z0-9_]`).
/// ASCII uppercase input is normalized to lowercase before validation; all
/// other characters, including whitespace and Unicode, are rejected.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Username(String);

/// An error returned when a username does not satisfy its domain rules.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UsernameError {
    /// The canonical username contains fewer than three characters.
    TooShort,
    /// The canonical username contains more than 32 characters.
    TooLong,
    /// The canonical username contains a character outside `[a-z0-9_]`.
    InvalidCharacter(char),
}

impl fmt::Display for UsernameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort => formatter.write_str("username must contain at least 3 characters"),
            Self::TooLong => formatter.write_str("username must contain at most 32 characters"),
            Self::InvalidCharacter(character) => {
                write!(
                    formatter,
                    "username contains invalid character {character:?}"
                )
            }
        }
    }
}

impl Error for UsernameError {}

impl Username {
    /// Returns the canonical lowercase username.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for Username {
    /// Returns the canonical lowercase username.
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl FromStr for Username {
    type Err = UsernameError;

    /// Normalizes and validates a username according to its domain rules.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let canonical = value.to_ascii_lowercase();
        if let Some(character) = canonical
            .chars()
            .find(|character| !matches!(character, 'a'..='z' | '0'..='9' | '_'))
        {
            return Err(UsernameError::InvalidCharacter(character));
        }

        let length = canonical.len();
        if length < 3 {
            return Err(UsernameError::TooShort);
        }
        if length > 32 {
            return Err(UsernameError::TooLong);
        }

        Ok(Self(canonical))
    }
}

impl fmt::Display for Username {
    /// Formats the canonical lowercase username.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::{Username, UsernameError};

    #[test]
    fn accepts_valid_usernames() {
        for value in ["abc", "otavio", "otavio_gonzaga", "user123"] {
            assert_eq!(Username::from_str(value).unwrap().as_str(), value);
        }
    }

    #[test]
    fn normalizes_ascii_case() {
        for value in ["Otavio", "OTAVIO", "oTaViO"] {
            assert_eq!(Username::from_str(value).unwrap().as_str(), "otavio");
        }
    }

    #[test]
    fn enforces_length_bounds() {
        assert_eq!(Username::from_str("ab"), Err(UsernameError::TooShort));
        assert!(Username::from_str(&"a".repeat(32)).is_ok());
        assert_eq!(
            Username::from_str(&"a".repeat(33)),
            Err(UsernameError::TooLong)
        );
    }

    #[test]
    fn rejects_invalid_characters_without_trimming() {
        for value in [
            "ota vio",
            "otavio-1",
            "otavio.name",
            "otavio@example",
            "otavio/name",
            r"otavio\name",
            " otavio",
            "otavio ",
            "ótavio",
            "ユーザー",
        ] {
            assert!(
                matches!(
                    Username::from_str(value),
                    Err(UsernameError::InvalidCharacter(_))
                ),
                "{value:?} should be rejected"
            );
        }
    }

    #[test]
    fn display_and_access_expose_the_canonical_value() {
        let username = Username::from_str("Otavio").unwrap();

        assert_eq!(username.to_string(), "otavio");
        assert_eq!(username.as_ref(), "otavio");
    }
}
