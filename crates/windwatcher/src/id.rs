use std::{error::Error, fmt, str::FromStr};

use uuid::{Uuid, Version};

/// An error returned when constructing a Windwatcher domain identifier.
///
/// Windwatcher identifiers are UUIDv7 values. A UUID with another version is
/// therefore rejected even when its textual representation is valid.
#[derive(Debug)]
pub enum IdentifierError {
    /// The supplied text was not a valid UUID.
    InvalidUuid(uuid::Error),
    /// The supplied UUID was valid but was not version 7.
    NotVersion7(Uuid),
}

impl fmt::Display for IdentifierError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUuid(_) => formatter.write_str("invalid UUID"),
            Self::NotVersion7(uuid) => write!(formatter, "UUID {uuid} is not version 7"),
        }
    }
}

impl Error for IdentifierError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidUuid(error) => Some(error),
            Self::NotVersion7(_) => None,
        }
    }
}

fn validate_uuid_v7(uuid: Uuid) -> Result<Uuid, IdentifierError> {
    if uuid.get_version() == Some(Version::SortRand) {
        Ok(uuid)
    } else {
        Err(IdentifierError::NotVersion7(uuid))
    }
}

macro_rules! domain_id {
    ($name:ident, $description:literal) => {
        #[doc = $description]
        ///
        /// This identifier always contains a UUIDv7. It can be generated
        /// without persistence coordination and explicitly converted to its
        /// underlying [`Uuid`].
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(Uuid);

        impl $name {
            /// Generates a new UUIDv7 identifier.
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }

            /// Returns this identifier's underlying UUIDv7.
            #[must_use]
            pub fn as_uuid(&self) -> Uuid {
                self.0
            }

            /// Consumes this identifier and returns its underlying UUIDv7.
            #[must_use]
            pub fn into_uuid(self) -> Uuid {
                self.0
            }
        }

        impl Default for $name {
            /// Generates a new UUIDv7 identifier.
            fn default() -> Self {
                Self::new()
            }
        }

        impl TryFrom<Uuid> for $name {
            type Error = IdentifierError;

            /// Constructs this identifier from a UUIDv7.
            fn try_from(uuid: Uuid) -> Result<Self, Self::Error> {
                validate_uuid_v7(uuid).map(Self)
            }
        }

        impl From<$name> for Uuid {
            /// Explicitly converts this domain identifier to its UUIDv7.
            fn from(identifier: $name) -> Self {
                identifier.into_uuid()
            }
        }

        impl FromStr for $name {
            type Err = IdentifierError;

            /// Parses a UUIDv7 domain identifier from its canonical UUID text.
            fn from_str(value: &str) -> Result<Self, Self::Err> {
                value
                    .parse::<Uuid>()
                    .map_err(IdentifierError::InvalidUuid)
                    .and_then(Self::try_from)
            }
        }

        impl fmt::Display for $name {
            /// Formats this identifier as its canonical UUID text.
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }
    };
}

domain_id!(UserId, "The UUIDv7 identifier of a [`crate::User`].");
domain_id!(SessionId, "The UUIDv7 identifier of a future user session.");
domain_id!(
    ConversationId,
    "The UUIDv7 identifier of a future messaging conversation."
);
domain_id!(MessageId, "The UUIDv7 identifier of a future message.");

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use uuid::{Uuid, Version};

    use super::{ConversationId, IdentifierError, MessageId, SessionId, UserId};

    macro_rules! identifier_tests {
        ($module:ident, $type:ident) => {
            mod $module {
                use super::*;

                #[test]
                fn generated_values_are_distinct_uuid_v7s() {
                    let first = $type::new();
                    let second = $type::new();

                    assert_eq!(first.as_uuid().get_version(), Some(Version::SortRand));
                    assert_eq!(second.as_uuid().get_version(), Some(Version::SortRand));
                    assert_ne!(first, second);
                }

                #[test]
                fn strings_round_trip() {
                    let identifier = $type::new();
                    let parsed = identifier.to_string().parse::<$type>().unwrap();

                    assert_eq!(parsed, identifier);
                }

                #[test]
                fn uuids_round_trip() {
                    let identifier = $type::new();
                    let converted = $type::try_from(identifier.into_uuid()).unwrap();

                    assert_eq!(converted, identifier);
                }

                #[test]
                fn malformed_text_is_rejected() {
                    let error = $type::from_str("not-a-uuid").unwrap_err();

                    assert!(matches!(error, IdentifierError::InvalidUuid(_)));
                }

                #[test]
                fn non_v7_uuid_is_rejected() {
                    let v4 = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
                    let error = $type::try_from(v4).unwrap_err();

                    assert!(matches!(error, IdentifierError::NotVersion7(_)));
                }

                #[test]
                fn ordering_follows_deterministic_v7_values() {
                    let earlier = $type::from_str("018f6f34-0000-7000-8000-000000000000").unwrap();
                    let later = $type::from_str("018f6f35-0000-7000-8000-000000000000").unwrap();

                    assert!(earlier < later);
                }
            }
        };
    }

    identifier_tests!(user_id, UserId);
    identifier_tests!(session_id, SessionId);
    identifier_tests!(conversation_id, ConversationId);
    identifier_tests!(message_id, MessageId);
}
