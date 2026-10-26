use serde::{Deserialize, Serialize};
use time::{OffsetDateTime, UtcOffset};
use utoipa::ToSchema;
use windwatcher::{ConversationMemberSummary, ConversationSummary, GroupRole, MembershipRole};

use super::super::{messages::dto::MessageResponse, users::dto::UserSummaryResponse};

#[derive(Deserialize, ToSchema)]
#[serde(untagged)]
#[schema(discriminator(property_name = "kind", mapping(
    ("direct" = "#/components/schemas/CreateDirectRequest"),
    ("group" = "#/components/schemas/CreateGroupRequest")
)))]
pub(in crate::http) enum CreateConversationRequest {
    Direct(CreateDirectRequest),
    Group(CreateGroupRequest),
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub(in crate::http) enum DirectKind {
    Direct,
}
#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub(in crate::http) enum GroupKind {
    Group,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(in crate::http) struct CreateDirectRequest {
    pub kind: DirectKind,
    #[schema(format = Uuid)]
    pub participant_id: String,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(in crate::http) struct CreateGroupRequest {
    pub kind: GroupKind,
    pub title: String,
    #[schema(value_type = Vec<String>, format = Uuid)]
    pub participant_ids: Vec<String>,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(in crate::http) struct UpdateConversationRequest {
    pub title: String,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub(in crate::http) enum AssignableRole {
    Admin,
    Member,
}
impl From<AssignableRole> for GroupRole {
    fn from(role: AssignableRole) -> Self {
        match role {
            AssignableRole::Admin => Self::Admin,
            AssignableRole::Member => Self::Member,
        }
    }
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(in crate::http) struct UpdateMemberRequest {
    pub role: AssignableRole,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(in crate::http) struct TransferOwnerRequest {
    #[schema(format = Uuid)]
    pub user_id: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub(in crate::http) enum RoleResponse {
    Direct,
    Owner,
    Admin,
    Member,
}
impl From<MembershipRole> for RoleResponse {
    fn from(role: MembershipRole) -> Self {
        match role {
            MembershipRole::Direct => Self::Direct,
            MembershipRole::Group(GroupRole::Owner) => Self::Owner,
            MembershipRole::Group(GroupRole::Admin) => Self::Admin,
            MembershipRole::Group(GroupRole::Member) => Self::Member,
        }
    }
}

#[derive(Serialize, ToSchema)]
pub(in crate::http) struct OwnMembershipResponse {
    role: RoleResponse,
    #[schema(format = Uuid)]
    last_read_message_id: Option<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub(in crate::http) enum ConversationKindResponse {
    Direct,
    Group,
}

#[derive(Serialize, ToSchema)]
pub(in crate::http) struct ConversationResponse {
    #[schema(format = Uuid)]
    id: String,
    kind: ConversationKindResponse,
    title: String,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    updated_at: OffsetDateTime,
    membership: OwnMembershipResponse,
    unread_count: u64,
    last_message: Option<MessageResponse>,
}
impl From<ConversationSummary> for ConversationResponse {
    fn from(summary: ConversationSummary) -> Self {
        Self {
            id: summary.conversation.id().to_string(),
            kind: match summary.conversation.kind() {
                windwatcher::ConversationKind::Direct => ConversationKindResponse::Direct,
                windwatcher::ConversationKind::Group => ConversationKindResponse::Group,
            },
            title: summary.title,
            created_at: summary.conversation.created_at().to_offset(UtcOffset::UTC),
            updated_at: summary.conversation.updated_at().to_offset(UtcOffset::UTC),
            membership: OwnMembershipResponse {
                role: summary.membership.role().into(),
                last_read_message_id: summary
                    .membership
                    .last_read_message_id()
                    .map(|id| id.to_string()),
            },
            unread_count: summary.unread_count,
            last_message: summary.last_message.map(Into::into),
        }
    }
}

#[derive(Serialize, ToSchema)]
pub(in crate::http) struct MemberResponse {
    user: UserSummaryResponse,
    role: RoleResponse,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    joined_at: OffsetDateTime,
}
impl From<ConversationMemberSummary> for MemberResponse {
    fn from(member: ConversationMemberSummary) -> Self {
        Self {
            user: member.user.into(),
            role: member.role.into(),
            joined_at: member.joined_at.to_offset(UtcOffset::UTC),
        }
    }
}
