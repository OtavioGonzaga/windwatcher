use time::OffsetDateTime;

use crate::{
    ConversationId, ConversationKind, ConversationReadState, GroupRole, GroupTitle, MessageView,
    UserId,
};

/// The committed value and meaningful facts produced by one application operation.
#[derive(Clone, Debug, PartialEq, Eq)]
#[must_use = "operation outcomes contain committed domain events that should be deliberately handled"]
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

/// A typed fact committed by a successful Windwatcher application operation.
///
/// Domain events are in-process library values. They are not serialized,
/// persisted, realtime payloads, queue jobs, or audit records.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DomainEvent {
    /// A Direct or Group conversation was created.
    ConversationCreated {
        /// Identity of the newly persisted conversation.
        conversation_id: ConversationId,
        /// Persisted conversation kind.
        kind: ConversationKind,
        /// Committed creation timestamp.
        occurred_at: OffsetDateTime,
    },
    /// A Group title changed.
    GroupRenamed {
        /// Identity of the renamed conversation.
        conversation_id: ConversationId,
        /// Persisted new Group title.
        title: GroupTitle,
        /// Committed conversation metadata timestamp produced by the rename.
        updated_at: OffsetDateTime,
    },
    /// An owner-only Group was deleted when its owner left.
    GroupDeleted {
        /// Identity of the hard-deleted conversation.
        conversation_id: ConversationId,
        /// Former Owner, carried because membership no longer exists after hard deletion and reactions may need an explicit recipient.
        former_owner_id: UserId,
        /// Time at which the deletion operation committed.
        occurred_at: OffsetDateTime,
    },
    /// A member joined a Group.
    GroupMemberAdded {
        /// Identity of the conversation whose membership changed.
        conversation_id: ConversationId,
        /// Identity of the newly added Member.
        user_id: UserId,
        /// Committed conversation metadata timestamp produced by the membership mutation.
        updated_at: OffsetDateTime,
    },
    /// A Group member's Admin or Member role changed.
    GroupMemberRoleChanged {
        /// Identity of the conversation whose membership changed.
        conversation_id: ConversationId,
        /// Identity of the member whose role changed.
        user_id: UserId,
        /// New Admin or Member role; ownership changes use `GroupOwnershipTransferred`.
        role: GroupRole,
        /// Committed conversation metadata timestamp produced by the membership mutation.
        updated_at: OffsetDateTime,
    },
    /// A Group member was removed or left.
    GroupMemberRemoved {
        /// Identity of the conversation whose membership changed.
        conversation_id: ConversationId,
        /// Identity of the removed or departing member, retained for terminal reactions.
        user_id: UserId,
        /// Committed conversation metadata timestamp produced by the membership mutation.
        updated_at: OffsetDateTime,
    },
    /// Group ownership changed atomically.
    GroupOwnershipTransferred {
        /// Identity of the conversation whose ownership changed.
        conversation_id: ConversationId,
        /// Owner who becomes Admin in the atomic transfer.
        previous_owner_id: UserId,
        /// Member who becomes Owner in the atomic transfer.
        new_owner_id: UserId,
        /// Committed conversation metadata timestamp produced by the transfer.
        updated_at: OffsetDateTime,
    },
    /// A message was sent.
    MessageSent {
        /// Committed application projection returned by persistence after the send.
        message: MessageView,
    },
    /// A message's content changed.
    MessageEdited {
        /// Committed application projection returned by persistence after the edit.
        message: MessageView,
    },
    /// A message became a tombstone.
    MessageDeleted {
        /// Committed tombstone projection returned by persistence after deletion.
        message: MessageView,
    },
    /// A user's conversation read state advanced.
    ConversationReadStateAdvanced {
        /// User whose own read state advanced; this is not a read receipt.
        user_id: UserId,
        /// Identity of the conversation whose read state changed.
        conversation_id: ConversationId,
        /// Authoritative read state after the operation; this is not a read receipt.
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
        let empty = Outcome::new(1);
        assert_eq!(*empty.value(), 1);
        assert!(empty.events().is_empty());
        assert_eq!(empty.into_value(), 1);
        let (value, events) = Outcome::with_event(1, event).into_parts();
        assert_eq!(value, 1);
        assert_eq!(events.len(), 1);
    }
}
