use super::{
    RealtimeService,
    event::{EventData, Kind},
};
use time::OffsetDateTime;
use windwatcher::{DomainEvent, GroupRole, MembershipRole};

fn realtime_type(event: &DomainEvent) -> &'static str {
    match event {
        DomainEvent::ConversationCreated { .. } => "conversation.created",
        DomainEvent::GroupRenamed { .. } => "conversation.updated",
        DomainEvent::GroupDeleted { .. } => "conversation.deleted",
        DomainEvent::GroupMemberAdded { .. } => "conversation.member.added",
        DomainEvent::GroupMemberRoleChanged { .. } => "conversation.member.updated",
        DomainEvent::GroupMemberRemoved { .. } => "conversation.member.removed",
        DomainEvent::GroupOwnershipTransferred { .. } => "conversation.owner.transferred",
        DomainEvent::MessageSent { .. } => "message.created",
        DomainEvent::MessageEdited { .. } => "message.updated",
        DomainEvent::MessageDeleted { .. } => "message.deleted",
        DomainEvent::ConversationReadStateAdvanced { .. } => "conversation.read_state.updated",
        _ => "unknown",
    }
}

impl RealtimeService {
    /// Translates committed library facts into the server-owned wire contract.
    pub(crate) fn react(&self, event: DomainEvent) {
        tracing::debug!(event_type = realtime_type(&event), "domain_event_reaction");
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

#[cfg(test)]
mod tests {
    use super::*;
    use windwatcher::{
        ConversationId, ConversationKind, ConversationReadState, GroupTitle, Message,
        MessageContent, MessageId, MessageView, UserId,
    };

    fn message() -> MessageView {
        let sender = UserId::new();
        MessageView {
            message: Message::new(
                MessageId::new(),
                ConversationId::new(),
                sender,
                MessageContent::new("message").unwrap(),
                None,
                OffsetDateTime::UNIX_EPOCH,
            )
            .unwrap(),
            sender_username: "sender".parse().unwrap(),
            reply_to: None,
        }
    }

    #[test]
    fn every_domain_event_maps_to_the_existing_realtime_type() {
        let conversation = ConversationId::new();
        let user = UserId::new();
        let event_types = [
            (
                DomainEvent::ConversationCreated {
                    conversation_id: conversation,
                    kind: ConversationKind::Group,
                    occurred_at: OffsetDateTime::UNIX_EPOCH,
                },
                "conversation.created",
            ),
            (
                DomainEvent::GroupRenamed {
                    conversation_id: conversation,
                    title: GroupTitle::new("group").unwrap(),
                    updated_at: OffsetDateTime::UNIX_EPOCH,
                },
                "conversation.updated",
            ),
            (
                DomainEvent::GroupDeleted {
                    conversation_id: conversation,
                    former_owner_id: user,
                    occurred_at: OffsetDateTime::UNIX_EPOCH,
                },
                "conversation.deleted",
            ),
            (
                DomainEvent::GroupMemberAdded {
                    conversation_id: conversation,
                    user_id: user,
                    updated_at: OffsetDateTime::UNIX_EPOCH,
                },
                "conversation.member.added",
            ),
            (
                DomainEvent::GroupMemberRoleChanged {
                    conversation_id: conversation,
                    user_id: user,
                    role: GroupRole::Admin,
                    updated_at: OffsetDateTime::UNIX_EPOCH,
                },
                "conversation.member.updated",
            ),
            (
                DomainEvent::GroupMemberRemoved {
                    conversation_id: conversation,
                    user_id: user,
                    updated_at: OffsetDateTime::UNIX_EPOCH,
                },
                "conversation.member.removed",
            ),
            (
                DomainEvent::GroupOwnershipTransferred {
                    conversation_id: conversation,
                    previous_owner_id: user,
                    new_owner_id: UserId::new(),
                    updated_at: OffsetDateTime::UNIX_EPOCH,
                },
                "conversation.owner.transferred",
            ),
            (
                DomainEvent::MessageSent { message: message() },
                "message.created",
            ),
            (
                DomainEvent::MessageEdited { message: message() },
                "message.updated",
            ),
            (
                DomainEvent::MessageDeleted { message: message() },
                "message.deleted",
            ),
            (
                DomainEvent::ConversationReadStateAdvanced {
                    user_id: user,
                    conversation_id: conversation,
                    state: ConversationReadState {
                        last_read_message_id: None,
                        unread_count: 0,
                    },
                },
                "conversation.read_state.updated",
            ),
        ];
        for (event, expected) in event_types {
            assert_eq!(realtime_type(&event), expected);
        }
    }
}
