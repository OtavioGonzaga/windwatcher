use super::{
    RealtimeService,
    event::{EventData, MemberChange, MemberPayload, RealtimeEvent},
};
use std::sync::Arc;
use time::OffsetDateTime;
use windwatcher::{ConversationId, ConversationStore, IdentityStore, MembershipRole, UserId};

impl RealtimeService {
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
    pub(crate) async fn notify_member(
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
        self.notify(conversation, data, now, &[]).await;
    }
    pub(crate) async fn notify_owner(
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
        self.notify(
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
    pub(crate) async fn notify(
        &self,
        conversation: ConversationId,
        data: EventData,
        now: OffsetDateTime,
        extra: &[UserId],
    ) {
        let event_type = data.event_type();
        match tokio::time::timeout(
            self.0.config.heartbeat_timeout,
            self.0.storage.member_ids(conversation),
        )
        .await
        {
            Ok(Ok(mut recipients)) => {
                recipients.extend_from_slice(extra);
                self.notify_users(&recipients, data, now);
            }
            _ => {
                tracing::warn!(conversation_id = %conversation, event_type, "audience_lookup_failure")
            }
        }
    }
    pub(crate) fn notify_users(&self, recipients: &[UserId], data: EventData, now: OffsetDateTime) {
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
