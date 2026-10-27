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
                    .dispatch_member(conversation, user, role, added, now)
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
        now: OffsetDateTime,
    ) {
        let service = self.clone();
        self.0.dispatch.submit(
            "conversation.owner.transferred",
            async move {
                service
                    .dispatch_owner(conversation, previous, next, now)
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
        let service = self.clone();
        let event_type = data.event_type();
        let extra = extra.to_vec();
        self.0.dispatch.submit(
            event_type,
            async move {
                service
                    .dispatch_conversation(conversation, data, now, &extra)
                    .await;
            },
            || {},
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
        now: OffsetDateTime,
    ) {
        let Some(member) = self.member_payload(conversation, user, role, now).await else {
            tracing::warn!(conversation_id = %conversation, "event_projection_failure");
            return;
        };
        let change = MemberChange {
            conversation_id: conversation.to_string(),
            member,
            conversation_updated_at: now,
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
                conversation_updated_at: now,
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
        let event_type = data.event_type();
        match self.0.storage.member_ids(conversation).await {
            Ok(mut recipients) => {
                recipients.extend_from_slice(extra);
                self.publish_users(&recipients, data, now);
            }
            _ => {
                tracing::warn!(conversation_id = %conversation, event_type, "audience_lookup_failure")
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
