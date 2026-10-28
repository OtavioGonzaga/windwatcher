use time::OffsetDateTime;

use crate::{
    ConversationId, ConversationKind, ConversationReadState, GroupRole, GroupTitle, MessageView,
    UserId,
};

/// The committed value and meaningful facts produced by one application operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome<T> {
    value: T,
    events: Vec<DomainEvent>,
}

impl<T> Outcome<T> {
    /// Creates an outcome without domain events.
    pub fn new(value: T) -> Self {
        Self {
            value,
            events: vec![],
        }
    }

    /// Creates an outcome with one domain event.
    pub fn with_event(value: T, event: DomainEvent) -> Self {
        Self {
            value,
            events: vec![event],
        }
    }

    /// Borrows the committed operation value.
    pub fn value(&self) -> &T {
        &self.value
    }

    /// Borrows the domain events emitted by the operation.
    pub fn events(&self) -> &[DomainEvent] {
        &self.events
    }

    /// Consumes the outcome and returns its value.
    pub fn into_value(self) -> T {
        self.value
    }

    /// Consumes the outcome and returns its value and events.
    pub fn into_parts(self) -> (T, Vec<DomainEvent>) {
        (self.value, self.events)
    }
}

impl<T> std::ops::Deref for Outcome<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<T: PartialEq> PartialEq<T> for Outcome<T> {
    fn eq(&self, other: &T) -> bool {
        self.value == *other
    }
}

/// A typed fact committed by a successful Windwatcher application operation.
///
/// Domain events are in-process library values. They are not serialized,
/// persisted, realtime payloads, queue jobs, or audit records.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[allow(missing_docs)]
pub enum DomainEvent {
    /// A Direct or Group conversation was created.
    ConversationCreated {
        conversation_id: ConversationId,
        kind: ConversationKind,
        occurred_at: OffsetDateTime,
    },
    /// A Group title changed.
    GroupRenamed {
        conversation_id: ConversationId,
        title: GroupTitle,
        updated_at: OffsetDateTime,
    },
    /// An owner-only Group was deleted when its owner left.
    GroupDeleted {
        conversation_id: ConversationId,
        former_owner_id: UserId,
        occurred_at: OffsetDateTime,
    },
    /// A member joined a Group.
    GroupMemberAdded {
        conversation_id: ConversationId,
        user_id: UserId,
        updated_at: OffsetDateTime,
    },
    /// A Group member's Admin or Member role changed.
    GroupMemberRoleChanged {
        conversation_id: ConversationId,
        user_id: UserId,
        role: GroupRole,
        updated_at: OffsetDateTime,
    },
    /// A Group member was removed or left.
    GroupMemberRemoved {
        conversation_id: ConversationId,
        user_id: UserId,
        updated_at: OffsetDateTime,
    },
    /// Group ownership changed atomically.
    GroupOwnershipTransferred {
        conversation_id: ConversationId,
        previous_owner_id: UserId,
        new_owner_id: UserId,
        updated_at: OffsetDateTime,
    },
    /// A message was sent.
    MessageSent { message: MessageView },
    /// A message's content changed.
    MessageEdited { message: MessageView },
    /// A message became a tombstone.
    MessageDeleted { message: MessageView },
    /// A user's conversation read state advanced.
    ConversationReadStateAdvanced {
        user_id: UserId,
        conversation_id: ConversationId,
        state: ConversationReadState,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ConversationId, ConversationKind};

    #[test]
    fn outcome_preserves_value_and_events() {
        let event = DomainEvent::ConversationCreated {
            conversation_id: ConversationId::new(),
            kind: ConversationKind::Direct,
            occurred_at: OffsetDateTime::UNIX_EPOCH,
        };
        assert!(Outcome::new(1).events().is_empty());
        let (value, events) = Outcome::with_event(1, event).into_parts();
        assert_eq!(value, 1);
        assert_eq!(events.len(), 1);
    }
}
