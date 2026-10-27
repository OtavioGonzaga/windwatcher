use serde::Serialize;
use time::OffsetDateTime;
use utoipa::ToSchema;

/// The wire contract is server-owned; identifiers are not engine event types.
#[derive(Serialize, ToSchema)]
pub(crate) struct RealtimeEvent {
    #[schema(format = Uuid)]
    pub event_id: String,
    pub schema_version: u32,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub occurred_at: OffsetDateTime,
    #[serde(flatten)]
    pub data: EventData,
}

#[derive(Serialize, ToSchema)]
#[serde(tag = "type", content = "payload")]
pub(crate) enum EventData {
    #[serde(rename = "message.created")]
    MessageCreated { message: MessagePayload },
    #[serde(rename = "message.updated")]
    MessageUpdated { message: MessagePayload },
    #[serde(rename = "message.deleted")]
    MessageDeleted { message: MessagePayload },
    #[serde(rename = "conversation.created")]
    ConversationCreated {
        #[schema(format = Uuid)]
        conversation_id: String,
        kind: Kind,
    },
    #[serde(rename = "conversation.updated")]
    ConversationUpdated {
        #[schema(format = Uuid)]
        conversation_id: String,
        title: String,
        #[serde(with = "time::serde::rfc3339")]
        #[schema(value_type = String, format = DateTime)]
        updated_at: OffsetDateTime,
    },
    #[serde(rename = "conversation.deleted")]
    ConversationDeleted {
        #[schema(format = Uuid)]
        conversation_id: String,
    },
    #[serde(rename = "conversation.member.added")]
    MemberAdded(MemberChange),
    #[serde(rename = "conversation.member.updated")]
    MemberUpdated(MemberChange),
    #[serde(rename = "conversation.member.removed")]
    MemberRemoved {
        #[schema(format = Uuid)]
        conversation_id: String,
        #[schema(format = Uuid)]
        user_id: String,
        #[serde(with = "time::serde::rfc3339")]
        #[schema(value_type = String, format = DateTime)]
        conversation_updated_at: OffsetDateTime,
    },
    #[serde(rename = "conversation.owner.transferred")]
    OwnerTransferred {
        #[schema(format = Uuid)]
        conversation_id: String,
        previous_owner: MemberPayload,
        new_owner: MemberPayload,
        #[serde(with = "time::serde::rfc3339")]
        #[schema(value_type = String, format = DateTime)]
        conversation_updated_at: OffsetDateTime,
    },
    #[serde(rename = "conversation.read_state.updated")]
    ReadStateUpdated {
        #[schema(format = Uuid)]
        conversation_id: String,
        #[schema(format = Uuid)]
        last_read_message_id: Option<String>,
        unread_count: u64,
    },
}

impl EventData {
    pub(super) fn event_type(&self) -> &'static str {
        match self {
            Self::MessageCreated { .. } => "message.created",
            Self::MessageUpdated { .. } => "message.updated",
            Self::MessageDeleted { .. } => "message.deleted",
            Self::ConversationCreated { .. } => "conversation.created",
            Self::ConversationUpdated { .. } => "conversation.updated",
            Self::ConversationDeleted { .. } => "conversation.deleted",
            Self::MemberAdded(_) => "conversation.member.added",
            Self::MemberUpdated(_) => "conversation.member.updated",
            Self::MemberRemoved { .. } => "conversation.member.removed",
            Self::OwnerTransferred { .. } => "conversation.owner.transferred",
            Self::ReadStateUpdated { .. } => "conversation.read_state.updated",
        }
    }
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Kind {
    Direct,
    Group,
}

#[derive(Serialize, ToSchema)]
pub(crate) struct MemberChange {
    #[schema(format = Uuid)]
    pub conversation_id: String,
    pub member: MemberPayload,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub conversation_updated_at: OffsetDateTime,
}

#[derive(Serialize, ToSchema)]
pub(crate) struct UserPayload {
    #[schema(format = Uuid)]
    pub id: String,
    pub username: String,
}
#[derive(Serialize, ToSchema)]
pub(crate) struct MemberPayload {
    pub user: UserPayload,
    pub role: Role,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub joined_at: OffsetDateTime,
}
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Role {
    Direct,
    Owner,
    Admin,
    Member,
}
impl From<windwatcher::ConversationMemberSummary> for MemberPayload {
    fn from(m: windwatcher::ConversationMemberSummary) -> Self {
        use windwatcher::{GroupRole as G, MembershipRole as M};
        Self {
            user: UserPayload {
                id: m.user.id.to_string(),
                username: m.user.username.to_string(),
            },
            role: match m.role {
                M::Direct => Role::Direct,
                M::Group(G::Owner) => Role::Owner,
                M::Group(G::Admin) => Role::Admin,
                M::Group(G::Member) => Role::Member,
            },
            joined_at: m.joined_at,
        }
    }
}

#[derive(Serialize, ToSchema)]
pub(crate) struct MessagePayload {
    #[schema(format = Uuid)]
    pub id: String,
    #[schema(format = Uuid)]
    pub conversation_id: String,
    #[schema(format = Uuid)]
    pub sender_id: String,
    pub sender_username: String,
    pub content: Option<String>,
    pub reply_to: Option<ReplyPayload>,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub edited_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub deleted_at: Option<OffsetDateTime>,
}
#[derive(Serialize, ToSchema)]
pub(crate) struct ReplyPayload {
    #[schema(format = Uuid)]
    pub id: String,
    #[schema(format = Uuid)]
    pub sender_id: String,
    pub sender_username: String,
    pub content: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub deleted_at: Option<OffsetDateTime>,
}
impl From<windwatcher::MessageView> for MessagePayload {
    fn from(m: windwatcher::MessageView) -> Self {
        Self {
            id: m.id().to_string(),
            conversation_id: m.conversation_id().to_string(),
            sender_id: m.sender_id().to_string(),
            sender_username: m.sender_username.to_string(),
            content: m.content().map(|c| c.as_str().to_owned()),
            created_at: m.created_at(),
            edited_at: m.edited_at(),
            deleted_at: m.deleted_at(),
            reply_to: m.reply_to.map(|r| ReplyPayload {
                id: r.id.to_string(),
                sender_id: r.sender.id.to_string(),
                sender_username: r.sender.username.to_string(),
                content: r.content.as_ref().map(|c| c.as_str().to_owned()),
                deleted_at: r.deleted_at,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    fn member(role: Role) -> MemberPayload {
        MemberPayload {
            user: UserPayload {
                id: uuid::Uuid::now_v7().to_string(),
                username: "alice".into(),
            },
            role,
            joined_at: OffsetDateTime::UNIX_EPOCH,
        }
    }
    fn message() -> MessagePayload {
        MessagePayload {
            id: uuid::Uuid::now_v7().to_string(),
            conversation_id: uuid::Uuid::now_v7().to_string(),
            sender_id: uuid::Uuid::now_v7().to_string(),
            sender_username: "alice".into(),
            content: Some("hello".into()),
            reply_to: None,
            created_at: OffsetDateTime::UNIX_EPOCH,
            edited_at: None,
            deleted_at: None,
        }
    }
    #[test]
    fn every_event_has_exact_envelope_and_payload_keys() {
        let id = uuid::Uuid::now_v7().to_string();
        let now = OffsetDateTime::UNIX_EPOCH;
        let change = || MemberChange {
            conversation_id: id.clone(),
            member: member(Role::Member),
            conversation_updated_at: now,
        };
        let cases = [
            (
                EventData::MessageCreated { message: message() },
                "message.created",
                vec!["message"],
            ),
            (
                EventData::MessageUpdated { message: message() },
                "message.updated",
                vec!["message"],
            ),
            (
                EventData::MessageDeleted { message: message() },
                "message.deleted",
                vec!["message"],
            ),
            (
                EventData::ConversationCreated {
                    conversation_id: id.clone(),
                    kind: Kind::Direct,
                },
                "conversation.created",
                vec!["conversation_id", "kind"],
            ),
            (
                EventData::ConversationUpdated {
                    conversation_id: id.clone(),
                    title: "Engineering".into(),
                    updated_at: now,
                },
                "conversation.updated",
                vec!["conversation_id", "title", "updated_at"],
            ),
            (
                EventData::ConversationDeleted {
                    conversation_id: id.clone(),
                },
                "conversation.deleted",
                vec!["conversation_id"],
            ),
            (
                EventData::MemberAdded(change()),
                "conversation.member.added",
                vec!["conversation_id", "member", "conversation_updated_at"],
            ),
            (
                EventData::MemberUpdated(change()),
                "conversation.member.updated",
                vec!["conversation_id", "member", "conversation_updated_at"],
            ),
            (
                EventData::MemberRemoved {
                    conversation_id: id.clone(),
                    user_id: id.clone(),
                    conversation_updated_at: now,
                },
                "conversation.member.removed",
                vec!["conversation_id", "user_id", "conversation_updated_at"],
            ),
            (
                EventData::OwnerTransferred {
                    conversation_id: id.clone(),
                    previous_owner: member(Role::Admin),
                    new_owner: member(Role::Owner),
                    conversation_updated_at: now,
                },
                "conversation.owner.transferred",
                vec![
                    "conversation_id",
                    "previous_owner",
                    "new_owner",
                    "conversation_updated_at",
                ],
            ),
            (
                EventData::ReadStateUpdated {
                    conversation_id: id.clone(),
                    last_read_message_id: Some(id.clone()),
                    unread_count: 0,
                },
                "conversation.read_state.updated",
                vec!["conversation_id", "last_read_message_id", "unread_count"],
            ),
        ];
        for (data, name, keys) in cases {
            assert_eq!(data.event_type(), name);
            let event = RealtimeEvent {
                event_id: id.clone(),
                schema_version: 1,
                occurred_at: now,
                data,
            };
            let value: Value = serde_json::to_value(event).unwrap();
            let mut envelope: Vec<_> = value
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            envelope.sort();
            assert_eq!(
                envelope,
                [
                    "event_id",
                    "occurred_at",
                    "payload",
                    "schema_version",
                    "type"
                ]
            );
            assert_eq!(value["type"], name);
            assert_eq!(value["schema_version"], 1);
            assert_eq!(value["occurred_at"], "1970-01-01T00:00:00Z");
            assert_eq!(
                uuid::Uuid::parse_str(value["event_id"].as_str().unwrap())
                    .unwrap()
                    .get_version_num(),
                7
            );
            let mut actual: Vec<_> = value["payload"]
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            actual.sort();
            let mut expected = keys;
            expected.sort();
            assert_eq!(actual, expected, "{name}");
            if value["payload"]["member"].is_object() {
                let m = &value["payload"]["member"];
                assert_eq!(m.as_object().unwrap().len(), 3);
                assert_eq!(m["joined_at"], "1970-01-01T00:00:00Z");
                assert_eq!(m["user"]["username"], "alice");
            }
        }
        assert_eq!(serde_json::to_value(Kind::Group).unwrap(), json!("group"));
    }
}
