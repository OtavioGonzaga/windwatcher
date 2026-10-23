use time::OffsetDateTime;

use crate::{UserId, Username};

/// A messaging-domain user identity.
///
/// Timestamps are UTC-aware [`OffsetDateTime`] instants supplied explicitly by
/// callers. The entity does not obtain the current time itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct User {
    id: UserId,
    username: Username,
    disabled_at: Option<OffsetDateTime>,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
}

impl User {
    /// Creates an enabled user with the supplied identity and creation instant.
    #[must_use]
    pub fn new(id: UserId, username: Username, now: OffsetDateTime) -> Self {
        Self {
            id,
            username,
            disabled_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Reconstructs a user loaded by a persistence adapter.
    ///
    /// Storage adapters must use this constructor rather than deriving state
    /// from a sequence of transitions, so persisted timestamps remain exact.
    #[must_use]
    pub fn restore(
        id: UserId,
        username: Username,
        disabled_at: Option<OffsetDateTime>,
        created_at: OffsetDateTime,
        updated_at: OffsetDateTime,
    ) -> Self {
        Self {
            id,
            username,
            disabled_at,
            created_at,
            updated_at,
        }
    }

    /// Returns the user's stable domain identifier.
    #[must_use]
    pub fn id(&self) -> UserId {
        self.id
    }

    /// Returns the user's canonical username.
    #[must_use]
    pub fn username(&self) -> &Username {
        &self.username
    }

    /// Returns whether this user is disabled.
    #[must_use]
    pub fn is_disabled(&self) -> bool {
        self.disabled_at.is_some()
    }

    /// Returns the instant at which this user was disabled, if it is disabled.
    #[must_use]
    pub fn disabled_at(&self) -> Option<OffsetDateTime> {
        self.disabled_at
    }

    /// Returns the instant at which this user was created.
    #[must_use]
    pub fn created_at(&self) -> OffsetDateTime {
        self.created_at
    }

    /// Returns the instant at which this user was last changed.
    #[must_use]
    pub fn updated_at(&self) -> OffsetDateTime {
        self.updated_at
    }

    /// Disables this user at `now`.
    ///
    /// Calling this method for an already disabled user is a no-op and does not
    /// rewrite its timestamps.
    pub fn disable(&mut self, now: OffsetDateTime) {
        if self.disabled_at.is_none() {
            self.disabled_at = Some(now);
            self.updated_at = now;
        }
    }

    /// Enables this user at `now`.
    ///
    /// Calling this method for an already enabled user is a no-op and does not
    /// rewrite its timestamps.
    pub fn enable(&mut self, now: OffsetDateTime) {
        if self.disabled_at.is_some() {
            self.disabled_at = None;
            self.updated_at = now;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use time::OffsetDateTime;

    use crate::{User, UserId, Username};

    fn timestamp(seconds: i64) -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(seconds).unwrap()
    }

    #[test]
    fn new_users_are_enabled() {
        let id = UserId::new();
        let username = Username::from_str("Otavio").unwrap();
        let now = timestamp(1_700_000_000);
        let user = User::new(id, username.clone(), now);

        assert_eq!(user.id(), id);
        assert_eq!(user.username(), &username);
        assert!(!user.is_disabled());
        assert_eq!(user.disabled_at(), None);
        assert_eq!(user.created_at(), now);
        assert_eq!(user.updated_at(), now);
    }

    #[test]
    fn disable_preserves_identity_and_creation() {
        let id = UserId::new();
        let username = Username::from_str("otavio").unwrap();
        let created_at = timestamp(1_700_000_000);
        let disabled_at = timestamp(1_700_000_100);
        let mut user = User::new(id, username.clone(), created_at);

        user.disable(disabled_at);

        assert!(user.is_disabled());
        assert_eq!(user.disabled_at(), Some(disabled_at));
        assert_eq!(user.updated_at(), disabled_at);
        assert_eq!(user.created_at(), created_at);
        assert_eq!(user.id(), id);
        assert_eq!(user.username(), &username);
    }

    #[test]
    fn repeated_disable_is_a_no_op() {
        let mut user = User::new(
            UserId::new(),
            Username::from_str("otavio").unwrap(),
            timestamp(1_700_000_000),
        );
        let first_disabled_at = timestamp(1_700_000_100);

        user.disable(first_disabled_at);
        user.disable(timestamp(1_700_000_200));

        assert_eq!(user.disabled_at(), Some(first_disabled_at));
        assert_eq!(user.updated_at(), first_disabled_at);
    }

    #[test]
    fn enable_updates_a_disabled_user() {
        let created_at = timestamp(1_700_000_000);
        let enabled_at = timestamp(1_700_000_200);
        let mut user = User::new(
            UserId::new(),
            Username::from_str("otavio").unwrap(),
            created_at,
        );
        user.disable(timestamp(1_700_000_100));

        user.enable(enabled_at);

        assert!(!user.is_disabled());
        assert_eq!(user.disabled_at(), None);
        assert_eq!(user.updated_at(), enabled_at);
        assert_eq!(user.created_at(), created_at);
    }

    #[test]
    fn repeated_enable_is_a_no_op() {
        let created_at = timestamp(1_700_000_000);
        let mut user = User::new(
            UserId::new(),
            Username::from_str("otavio").unwrap(),
            created_at,
        );

        user.enable(timestamp(1_700_000_100));

        assert_eq!(user.updated_at(), created_at);
    }
}
