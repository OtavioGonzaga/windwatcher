use time::OffsetDateTime;

use crate::{ConversationId, MessageId, UserId};

/// The two supported conversation lifecycles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConversationKind {
    /// An immutable, unordered pair of participants.
    Direct,
    /// A mutable conversation with exactly one owner.
    Group,
}

/// A validated group title.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupTitle(String);

/// A title failed the group-title invariant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum GroupTitleError {
    /// Group titles cannot be empty or whitespace-only.
    #[error("group title must not be blank")]
    Blank,
}

impl GroupTitle {
    /// Validates and preserves an exact group title.
    pub fn new(value: impl Into<String>) -> Result<Self, GroupTitleError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(GroupTitleError::Blank);
        }
        Ok(Self(value))
    }

    /// Returns the exact accepted title.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A conversation and its immutable metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conversation {
    id: ConversationId,
    kind: ConversationKind,
    title: Option<GroupTitle>,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
}

/// A persisted conversation violated its kind-specific invariants.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ConversationError {
    /// A Direct conversation had a title.
    #[error("direct conversations cannot have a title")]
    DirectTitle,
    /// A Group conversation lacked its required title.
    #[error("group conversations require a title")]
    GroupTitleMissing,
}

impl Conversation {
    /// Constructs a Direct conversation with immutable, untitled metadata.
    pub fn new_direct(id: ConversationId, now: OffsetDateTime) -> Self {
        Self {
            id,
            kind: ConversationKind::Direct,
            title: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Constructs a Group conversation.
    pub fn new_group(id: ConversationId, title: GroupTitle, now: OffsetDateTime) -> Self {
        Self {
            id,
            kind: ConversationKind::Group,
            title: Some(title),
            created_at: now,
            updated_at: now,
        }
    }

    /// Reconstructs and validates a conversation read from persistence.
    pub fn restore(
        id: ConversationId,
        kind: ConversationKind,
        title: Option<GroupTitle>,
        created_at: OffsetDateTime,
        updated_at: OffsetDateTime,
    ) -> Result<Self, ConversationError> {
        match (kind, title.is_some()) {
            (ConversationKind::Direct, true) => Err(ConversationError::DirectTitle),
            (ConversationKind::Group, false) => Err(ConversationError::GroupTitleMissing),
            _ => Ok(Self {
                id,
                kind,
                title,
                created_at,
                updated_at,
            }),
        }
    }

    /// Returns this conversation's identifier.
    pub fn id(&self) -> ConversationId {
        self.id
    }
    /// Returns its lifecycle kind.
    pub fn kind(&self) -> ConversationKind {
        self.kind
    }
    /// Returns a Group title, if this is a Group.
    pub fn title(&self) -> Option<&GroupTitle> {
        self.title.as_ref()
    }
    /// Returns the creation time.
    pub fn created_at(&self) -> OffsetDateTime {
        self.created_at
    }
    /// Returns the last meaningful mutation time.
    pub fn updated_at(&self) -> OffsetDateTime {
        self.updated_at
    }
}

/// A Group-specific membership role.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupRole {
    /// The sole Group owner.
    Owner,
    /// A Group administrator.
    Admin,
    /// A regular Group member.
    Member,
}

/// A role valid only for its associated conversation kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MembershipRole {
    /// Membership in a Direct conversation.
    Direct,
    /// Membership in a Group conversation.
    Group(GroupRole),
}

/// A user's current membership in a conversation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversationMember {
    conversation_id: ConversationId,
    user_id: UserId,
    role: MembershipRole,
    joined_at: OffsetDateTime,
    last_read_message_id: Option<MessageId>,
}
impl ConversationMember {
    /// Reconstructs a membership after adapter validation.
    pub fn new(
        conversation_id: ConversationId,
        user_id: UserId,
        role: MembershipRole,
        joined_at: OffsetDateTime,
    ) -> Self {
        Self {
            conversation_id,
            user_id,
            role,
            joined_at,
            last_read_message_id: None,
        }
    }
    /// Restores a membership and its persisted UUIDv7 read boundary.
    pub fn restore(
        conversation_id: ConversationId,
        user_id: UserId,
        role: MembershipRole,
        joined_at: OffsetDateTime,
        last_read_message_id: Option<MessageId>,
    ) -> Self {
        Self {
            conversation_id,
            user_id,
            role,
            joined_at,
            last_read_message_id,
        }
    }
    /// Returns the explicit monotonic read watermark, if any.
    pub fn last_read_message_id(&self) -> Option<MessageId> {
        self.last_read_message_id
    }
    /// Returns the conversation identifier.
    pub fn conversation_id(&self) -> ConversationId {
        self.conversation_id
    }
    /// Returns the member identifier.
    pub fn user_id(&self) -> UserId {
        self.user_id
    }
    /// Returns this member's role.
    pub fn role(&self) -> MembershipRole {
        self.role
    }
    /// Returns when the member joined.
    pub fn joined_at(&self) -> OffsetDateTime {
        self.joined_at
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::OffsetDateTime;

    #[test]
    fn constructors_preserve_kind_invariants() {
        let now = OffsetDateTime::UNIX_EPOCH;
        let id = ConversationId::new();
        assert!(
            Conversation::restore(
                id,
                ConversationKind::Direct,
                Some(GroupTitle::new("x").unwrap()),
                now,
                now
            )
            .is_err()
        );
        assert!(Conversation::restore(id, ConversationKind::Group, None, now, now).is_err());
        assert_eq!(GroupTitle::new("  "), Err(GroupTitleError::Blank));
    }
}
