use serde::{Deserialize, Serialize};
use time::{OffsetDateTime, UtcOffset};
use utoipa::ToSchema;
use windwatcher::{MessageView, ReplyPreview};

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(in crate::http) struct SendMessageRequest {
    pub content: String,
    #[schema(format = Uuid)]
    pub reply_to_message_id: Option<String>,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(in crate::http) struct EditMessageRequest {
    pub content: String,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(in crate::http) struct ReadStateRequest {
    #[schema(format = Uuid)]
    pub last_read_message_id: String,
}

#[derive(Serialize, ToSchema)]
pub(in crate::http) struct ReadStateResponse {
    #[schema(format = Uuid)]
    pub last_read_message_id: Option<String>,
    pub unread_count: u64,
}
impl From<windwatcher::ConversationReadState> for ReadStateResponse {
    fn from(state: windwatcher::ConversationReadState) -> Self {
        Self {
            last_read_message_id: state.last_read_message_id.map(|id| id.to_string()),
            unread_count: state.unread_count,
        }
    }
}

#[derive(Serialize, ToSchema)]
pub(in crate::http) struct ReplyPreviewResponse {
    #[schema(format = Uuid)]
    id: String,
    #[schema(format = Uuid)]
    sender_id: String,
    sender_username: String,
    content: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    #[schema(value_type = Option<String>, format = DateTime)]
    deleted_at: Option<OffsetDateTime>,
}
impl From<ReplyPreview> for ReplyPreviewResponse {
    fn from(reply: ReplyPreview) -> Self {
        Self {
            id: reply.id.to_string(),
            sender_id: reply.sender.id.to_string(),
            sender_username: reply.sender.username.to_string(),
            content: reply.content.map(|c| c.as_str().to_owned()),
            deleted_at: reply.deleted_at.map(|t| t.to_offset(UtcOffset::UTC)),
        }
    }
}

#[derive(Serialize, ToSchema)]
pub(in crate::http) struct MessageResponse {
    #[schema(format = Uuid)]
    id: String,
    #[schema(format = Uuid)]
    conversation_id: String,
    #[schema(format = Uuid)]
    sender_id: String,
    sender_username: String,
    content: Option<String>,
    reply_to: Option<ReplyPreviewResponse>,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    #[schema(value_type = Option<String>, format = DateTime)]
    edited_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    #[schema(value_type = Option<String>, format = DateTime)]
    deleted_at: Option<OffsetDateTime>,
}
impl From<MessageView> for MessageResponse {
    fn from(view: MessageView) -> Self {
        Self {
            id: view.id().to_string(),
            conversation_id: view.conversation_id().to_string(),
            sender_id: view.sender_id().to_string(),
            content: view.content().map(|c| c.as_str().to_owned()),
            created_at: view.created_at().to_offset(UtcOffset::UTC),
            edited_at: view.edited_at().map(|t| t.to_offset(UtcOffset::UTC)),
            deleted_at: view.deleted_at().map(|t| t.to_offset(UtcOffset::UTC)),
            sender_username: view.sender_username.to_string(),
            reply_to: view.reply_to.map(|reply| (*reply).into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windwatcher::*;
    #[test]
    fn realtime_message_contract_matches_http_including_reply_tombstones() {
        let now = OffsetDateTime::UNIX_EPOCH;
        for deleted in [false, true] {
            let reply_id = MessageId::new();
            let view = MessageView {
                message: Message::restore(
                    MessageId::new(),
                    ConversationId::new(),
                    UserId::new(),
                    (!deleted).then(|| MessageContent::new("hello").unwrap()),
                    Some(reply_id),
                    now,
                    Some(now),
                    deleted.then_some(now),
                )
                .unwrap(),
                sender_username: "alice".parse().unwrap(),
                reply_to: Some(Box::new(ReplyPreview {
                    id: reply_id,
                    sender: UserSummary {
                        id: UserId::new(),
                        username: "bob".parse().unwrap(),
                    },
                    content: None,
                    deleted_at: Some(now),
                })),
            };
            let http = serde_json::to_value(MessageResponse::from(view.clone())).unwrap();
            let realtime =
                serde_json::to_value(crate::realtime::event::MessagePayload::from(view)).unwrap();
            assert_eq!(http, realtime);
            assert_eq!(http.as_object().unwrap().len(), 9);
        }
    }
}
