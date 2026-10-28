use super::{
    RealtimeService,
    event::{EventData, Kind},
};
use time::OffsetDateTime;
use windwatcher::{DomainEvent, GroupRole, MembershipRole};

impl RealtimeService {
    /// Translates committed library facts into the server-owned wire contract.
    pub(crate) fn react(&self, event: DomainEvent) {
        match event {
            DomainEvent::ConversationCreated {
                conversation_id,
                kind,
                occurred_at,
            } => self.notify(
                conversation_id,
                EventData::ConversationCreated {
                    conversation_id: conversation_id.to_string(),
                    kind: match kind {
                        windwatcher::ConversationKind::Direct => Kind::Direct,
                        windwatcher::ConversationKind::Group => Kind::Group,
                    },
                },
                occurred_at,
                &[],
            ),
            DomainEvent::GroupRenamed {
                conversation_id,
                title,
                updated_at,
            } => self.notify(
                conversation_id,
                EventData::ConversationUpdated {
                    conversation_id: conversation_id.to_string(),
                    title: title.as_str().to_owned(),
                    updated_at,
                },
                updated_at,
                &[],
            ),
            DomainEvent::GroupDeleted {
                conversation_id,
                former_owner_id,
                occurred_at,
            } => self.notify_users(
                &[former_owner_id],
                EventData::ConversationDeleted {
                    conversation_id: conversation_id.to_string(),
                },
                occurred_at,
            ),
            DomainEvent::GroupMemberAdded {
                conversation_id,
                user_id,
                updated_at,
            } => self.notify_member(
                conversation_id,
                user_id,
                MembershipRole::Group(GroupRole::Member),
                true,
                updated_at,
                updated_at,
            ),
            DomainEvent::GroupMemberRoleChanged {
                conversation_id,
                user_id,
                role,
                updated_at,
            } => self.notify_member(
                conversation_id,
                user_id,
                MembershipRole::Group(role),
                false,
                updated_at,
                updated_at,
            ),
            DomainEvent::GroupMemberRemoved {
                conversation_id,
                user_id,
                updated_at,
            } => self.notify(
                conversation_id,
                EventData::MemberRemoved {
                    conversation_id: conversation_id.to_string(),
                    user_id: user_id.to_string(),
                    conversation_updated_at: updated_at,
                },
                updated_at,
                &[user_id],
            ),
            DomainEvent::GroupOwnershipTransferred {
                conversation_id,
                previous_owner_id,
                new_owner_id,
                updated_at,
            } => self.notify_owner(
                conversation_id,
                previous_owner_id,
                new_owner_id,
                updated_at,
                updated_at,
            ),
            DomainEvent::MessageSent { message } => self.notify(
                message.conversation_id(),
                EventData::MessageCreated {
                    message: message.clone().into(),
                },
                message.created_at(),
                &[],
            ),
            DomainEvent::MessageEdited { message } => self.notify(
                message.conversation_id(),
                EventData::MessageUpdated {
                    message: message.clone().into(),
                },
                message.edited_at().unwrap_or_else(OffsetDateTime::now_utc),
                &[],
            ),
            DomainEvent::MessageDeleted { message } => self.notify(
                message.conversation_id(),
                EventData::MessageDeleted {
                    message: message.clone().into(),
                },
                message.deleted_at().unwrap_or_else(OffsetDateTime::now_utc),
                &[],
            ),
            DomainEvent::ConversationReadStateAdvanced {
                user_id,
                conversation_id,
                state,
            } => self.notify_users(
                &[user_id],
                EventData::ReadStateUpdated {
                    conversation_id: conversation_id.to_string(),
                    last_read_message_id: state.last_read_message_id.map(|id| id.to_string()),
                    unread_count: state.unread_count,
                },
                OffsetDateTime::now_utc(),
            ),
            _ => tracing::warn!("unknown_domain_event"),
        }
    }

    pub(crate) fn react_all(&self, events: impl IntoIterator<Item = DomainEvent>) {
        for event in events {
            self.react(event);
        }
    }
}
