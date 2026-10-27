use super::{
    RealtimeService,
    event::{EventData, MemberChange, MemberPayload, RealtimeEvent},
};
use std::sync::Arc;
use time::OffsetDateTime;
use windwatcher::{ConversationId, ConversationStore, IdentityStore, MembershipRole, UserId};

impl RealtimeService {
    pub(crate) fn notify_member(
        &self,
        conversation: ConversationId,
        user: UserId,
        role: MembershipRole,
        added: bool,
        updated_at: OffsetDateTime,
        now: OffsetDateTime,
    ) {
        let service = self.clone();
        let event_type = if added {
            "conversation.member.added"
        } else {
            "conversation.member.updated"
        };
        self.0.dispatch.submit(
            event_type,
            async move {
                service
                    .dispatch_member(conversation, user, role, added, updated_at, now)
                    .await;
            },
            || {},
        );
    }

    pub(crate) fn notify_owner(
        &self,
        conversation: ConversationId,
        previous: UserId,
        next: UserId,
        updated_at: OffsetDateTime,
        now: OffsetDateTime,
    ) {
        let service = self.clone();
        self.0.dispatch.submit(
            "conversation.owner.transferred",
            async move {
                service
                    .dispatch_owner(conversation, previous, next, updated_at, now)
                    .await;
            },
            || {},
        );
    }

    pub(crate) fn notify(
        &self,
        conversation: ConversationId,
        data: EventData,
        now: OffsetDateTime,
        extra: &[UserId],
    ) {
        let storage = self.0.storage.clone();
        self.submit_conversation(conversation, data, now, extra, async move {
            storage.member_ids(conversation).await
        });
    }

    fn submit_conversation(
        &self,
        conversation: ConversationId,
        data: EventData,
        now: OffsetDateTime,
        extra: &[UserId],
        lookup: impl std::future::Future<
            Output = Result<Vec<UserId>, windwatcher::ConversationStoreError>,
        > + Send
        + 'static,
    ) {
        let service = self.clone();
        let event_type = data.event_type();
        // Only terminal removal has an explicit audience independent of current membership.
        let fallback = match &data {
            EventData::MemberRemoved {
                conversation_id,
                user_id,
                conversation_updated_at,
            } if !extra.is_empty() => Some(EventData::MemberRemoved {
                conversation_id: conversation_id.clone(),
                user_id: user_id.clone(),
                conversation_updated_at: *conversation_updated_at,
            }),
            _ => None,
        };
        let terminal_service = self.clone();
        let terminal_recipients = extra.to_vec();
        let extra = extra.to_vec();
        self.0.dispatch.submit(
            event_type,
            async move {
                let recipients = lookup.await;
                service.publish_conversation(conversation, data, now, &extra, recipients);
            },
            move || {
                if let Some(data) = fallback {
                    terminal_service.publish_users(&terminal_recipients, data, now);
                }
            },
        );
    }

    pub(crate) fn notify_users(&self, recipients: &[UserId], data: EventData, now: OffsetDateTime) {
        let service = self.clone();
        let recipients = recipients.to_vec();
        self.0.dispatch.submit(
            data.event_type(),
            async move {
                service.publish_users(&recipients, data, now);
            },
            || {},
        );
    }
    async fn member_payload(
        &self,
        conversation: ConversationId,
        user: UserId,
        role: MembershipRole,
        now: OffsetDateTime,
    ) -> Option<MemberPayload> {
        // Attribution and joined_at are immutable for this membership. Reject a
        // replacement membership created after the mutation instead of fabricating history.
        let member = self
            .0
            .storage
            .find_member(conversation, user)
            .await
            .ok()
            .flatten()?;
        if member.joined_at() > now {
            return None;
        }
        let user = self.0.storage.find_user(user).await.ok().flatten()?;
        let summary = windwatcher::ConversationMemberSummary {
            user: windwatcher::UserSummary {
                id: user.id(),
                username: user.username().clone(),
            },
            role,
            joined_at: member.joined_at(),
        };
        Some(summary.into())
    }
    async fn dispatch_member(
        &self,
        conversation: ConversationId,
        user: UserId,
        role: MembershipRole,
        added: bool,
        updated_at: OffsetDateTime,
        now: OffsetDateTime,
    ) {
        let Some(member) = self.member_payload(conversation, user, role, now).await else {
            tracing::warn!(conversation_id = %conversation, "event_projection_failure");
            return;
        };
        let change = MemberChange {
            conversation_id: conversation.to_string(),
            member,
            conversation_updated_at: updated_at,
        };
        let data = if added {
            EventData::MemberAdded(change)
        } else {
            EventData::MemberUpdated(change)
        };
        self.dispatch_conversation(conversation, data, now, &[])
            .await;
    }
    async fn dispatch_owner(
        &self,
        conversation: ConversationId,
        previous: UserId,
        next: UserId,
        updated_at: OffsetDateTime,
        now: OffsetDateTime,
    ) {
        use windwatcher::GroupRole;
        let (Some(previous_owner), Some(new_owner)) = (
            self.member_payload(
                conversation,
                previous,
                MembershipRole::Group(GroupRole::Admin),
                now,
            )
            .await,
            self.member_payload(
                conversation,
                next,
                MembershipRole::Group(GroupRole::Owner),
                now,
            )
            .await,
        ) else {
            tracing::warn!(conversation_id = %conversation, "event_projection_failure");
            return;
        };
        self.dispatch_conversation(
            conversation,
            EventData::OwnerTransferred {
                conversation_id: conversation.to_string(),
                previous_owner,
                new_owner,
                conversation_updated_at: updated_at,
            },
            now,
            &[],
        )
        .await;
    }
    async fn dispatch_conversation(
        &self,
        conversation: ConversationId,
        data: EventData,
        now: OffsetDateTime,
        extra: &[UserId],
    ) {
        let recipients = self.0.storage.member_ids(conversation).await;
        self.publish_conversation(conversation, data, now, extra, recipients);
    }

    fn publish_conversation(
        &self,
        conversation: ConversationId,
        data: EventData,
        now: OffsetDateTime,
        extra: &[UserId],
        recipients: Result<Vec<UserId>, windwatcher::ConversationStoreError>,
    ) {
        let event_type = data.event_type();
        match recipients {
            Ok(mut recipients) => {
                recipients.extend_from_slice(extra);
                self.publish_users(&recipients, data, now);
            }
            _ => {
                tracing::warn!(conversation_id = %conversation, event_type, explicit_recipient_count = extra.len(), "audience_lookup_failure");
                if !extra.is_empty() {
                    self.publish_users(extra, data, now);
                }
            }
        }
    }
    fn publish_users(&self, recipients: &[UserId], data: EventData, now: OffsetDateTime) {
        let event_type = data.event_type();
        let event = RealtimeEvent {
            event_id: uuid::Uuid::now_v7().to_string(),
            schema_version: 1,
            occurred_at: now.to_offset(time::UtcOffset::UTC),
            data,
        };
        match serde_json::to_string(&event) {
            Ok(frame) => {
                let delivered = self.0.hub.publish(recipients, Arc::from(frame));
                tracing::debug!(event_id = %event.event_id, event_type, recipient_count = recipients.len(), delivered, "event_published");
            }
            Err(_) => {
                tracing::warn!(event_id = %event.event_id, event_type, "event_serialization_failure")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn terminal_recipient_survives_whole_dispatch_deadline() {
        let (_directory, service, session) = crate::realtime::tests::fixture().await;
        let mut connection = service.0.hub.register(session.user_id(), session.id(), 2);
        service.0.hub.activate(connection.guard.id);
        tokio::time::pause();
        let conversation = ConversationId::new();
        let now = OffsetDateTime::now_utc();
        let data = EventData::MemberRemoved {
            conversation_id: conversation.to_string(),
            user_id: session.user_id().to_string(),
            conversation_updated_at: now,
        };
        let (entered, waiting) = tokio::sync::oneshot::channel();
        service.submit_conversation(conversation, data, now, &[session.user_id()], async {
            entered.send(()).unwrap();
            std::future::pending().await
        });
        waiting.await.unwrap();
        tokio::time::advance(service.0.config.dispatch_timeout).await;
        let frame = connection.events.recv().await.unwrap();
        let value: serde_json::Value = serde_json::from_str(&frame).unwrap();
        assert_eq!(value["type"], "conversation.member.removed");
        assert_eq!(value["payload"]["user_id"], session.user_id().to_string());
        assert!(connection.events.try_recv().is_err());
    }
}
