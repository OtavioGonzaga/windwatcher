use std::{error::Error, fmt};

use mongodb::{
    bson::{Binary, Bson, Document, doc, spec::BinarySubtype},
    error::ErrorKind,
};
use time::OffsetDateTime;

use crate::{
    AddMemberStoreOutcome, ChangeStoreOutcome, Conversation, ConversationId, ConversationKind,
    ConversationMember, ConversationStore, ConversationStoreError, GroupRole, GroupTitle,
    LeaveStoreOutcome, MembershipRole, MessageId, StartDirectStoreOutcome, UserId,
};

use super::{MongoDbStorage, transactions};

#[derive(Debug)]
struct Invalid(&'static str);
impl fmt::Display for Invalid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl Error for Invalid {}
fn bad(message: &'static str) -> ConversationStoreError {
    ConversationStoreError::Unexpected(Box::new(Invalid(message)))
}
fn unavailable(error: &mongodb::error::Error) -> bool {
    error.contains_label("TransientTransactionError")
        || error.contains_label("UnknownTransactionCommitResult")
        || matches!(
            error.kind.as_ref(),
            ErrorKind::Io(_)
                | ErrorKind::ConnectionPoolCleared { .. }
                | ErrorKind::ServerSelection { .. }
        )
}
fn db_error(error: mongodb::error::Error) -> ConversationStoreError {
    if unavailable(&error) {
        ConversationStoreError::Unavailable(Box::new(error))
    } else if matches!(error.kind.as_ref(), ErrorKind::Write(mongodb::error::WriteFailure::WriteError(write)) if write.code == 11000)
        || matches!(error.kind.as_ref(), ErrorKind::Command(command) if command.code == 11000 || command.code == 121)
    {
        ConversationStoreError::Conflict
    } else {
        ConversationStoreError::Unexpected(Box::new(error))
    }
}
fn id(value: uuid::Uuid) -> Binary {
    Binary {
        subtype: BinarySubtype::Uuid,
        bytes: value.as_bytes().to_vec(),
    }
}
fn uuid(document: &Document, key: &str) -> Result<uuid::Uuid, ConversationStoreError> {
    let Some(Bson::Binary(value)) = document.get(key) else {
        return Err(bad("persisted UUID is not BSON binary"));
    };
    if value.subtype != BinarySubtype::Uuid || value.bytes.len() != 16 {
        return Err(bad("persisted UUID has invalid subtype or length"));
    }
    uuid::Uuid::from_slice(&value.bytes).map_err(|_| bad("invalid persisted UUID"))
}
fn micros(value: OffsetDateTime) -> Result<i64, ConversationStoreError> {
    i64::try_from(value.unix_timestamp_nanos().div_euclid(1_000))
        .map_err(|_| bad("timestamp out of range"))
}
fn date(value: &Document, key: &str) -> Result<OffsetDateTime, ConversationStoreError> {
    let micros = value
        .get_i64(key)
        .map_err(|_| bad("persisted timestamp is not BSON Int64"))?;
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(micros) * 1_000)
        .map_err(|_| bad("invalid persisted timestamp"))
}
fn normalized(value: OffsetDateTime) -> Result<OffsetDateTime, ConversationStoreError> {
    let micros = micros(value)?;
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(micros) * 1_000)
        .map_err(|_| bad("invalid timestamp"))
}
pub(super) fn read_conversation(
    document: &Document,
) -> Result<Conversation, ConversationStoreError> {
    let id = ConversationId::try_from(uuid(document, "_id")?)
        .map_err(|_| bad("invalid conversation ID"))?;
    let kind = match document
        .get_str("kind")
        .map_err(|_| bad("persisted conversation kind is invalid"))?
    {
        "direct" => ConversationKind::Direct,
        "group" => ConversationKind::Group,
        _ => return Err(bad("persisted conversation kind is invalid")),
    };
    let title = match document.get("title") {
        None | Some(Bson::Null) => None,
        Some(Bson::String(value)) => {
            Some(GroupTitle::new(value.clone()).map_err(|_| bad("invalid persisted group title"))?)
        }
        _ => return Err(bad("persisted conversation title has invalid BSON type")),
    };
    Conversation::restore(
        id,
        kind,
        title,
        date(document, "created_at")?,
        date(document, "updated_at")?,
    )
    .map_err(|_| bad("incoherent persisted conversation"))
}
fn role(value: &str) -> Result<MembershipRole, ConversationStoreError> {
    match value {
        "direct" => Ok(MembershipRole::Direct),
        "owner" => Ok(MembershipRole::Group(GroupRole::Owner)),
        "admin" => Ok(MembershipRole::Group(GroupRole::Admin)),
        "member" => Ok(MembershipRole::Group(GroupRole::Member)),
        _ => Err(bad("invalid persisted membership role")),
    }
}
pub(super) fn read_member(
    document: &Document,
) -> Result<ConversationMember, ConversationStoreError> {
    let conversation = ConversationId::try_from(uuid(document, "conversation_id")?)
        .map_err(|_| bad("invalid conversation ID"))?;
    let user = UserId::try_from(uuid(document, "user_id")?).map_err(|_| bad("invalid user ID"))?;
    let role = role(
        document
            .get_str("role")
            .map_err(|_| bad("persisted membership role is invalid"))?,
    )?;
    let marker = match document.get("last_read_message_id") {
        None | Some(Bson::Null) => None,
        Some(Bson::Binary(value))
            if value.subtype == BinarySubtype::Uuid && value.bytes.len() == 16 =>
        {
            Some(
                MessageId::try_from(
                    uuid::Uuid::from_slice(&value.bytes).map_err(|_| bad("invalid read marker"))?,
                )
                .map_err(|_| bad("invalid read marker"))?,
            )
        }
        _ => return Err(bad("persisted read marker has invalid BSON representation")),
    };
    Ok(ConversationMember::restore(
        conversation,
        user,
        role,
        date(document, "joined_at")?,
        marker,
    ))
}
pub(super) fn direct_key(a: UserId, b: UserId) -> Binary {
    let (mut left, mut right) = (*a.as_uuid().as_bytes(), *b.as_uuid().as_bytes());
    if left > right {
        std::mem::swap(&mut left, &mut right);
    }
    Binary {
        subtype: BinarySubtype::Generic,
        bytes: [left.as_slice(), right.as_slice()].concat(),
    }
}
pub(super) fn conv_doc(
    conversation: &Conversation,
    direct: Option<Binary>,
) -> Result<Document, ConversationStoreError> {
    Ok(doc! {
        "_id": id(conversation.id().as_uuid()),
        "kind": if conversation.kind() == ConversationKind::Direct { "direct" } else { "group" },
        "title": conversation.title().map(|title| Bson::String(title.as_str().to_owned())).unwrap_or(Bson::Null),
        "direct_key": direct.map(Bson::Binary).unwrap_or(Bson::Null),
        "created_at": micros(conversation.created_at())?,
        "updated_at": micros(conversation.updated_at())?,
        "_ww_revision": 0_i64,
    })
}
pub(super) fn member_doc(
    conversation: ConversationId,
    user: UserId,
    role: &str,
    joined_at: OffsetDateTime,
    marker: Option<MessageId>,
) -> Result<Document, ConversationStoreError> {
    Ok(doc! {
        "conversation_id": id(conversation.as_uuid()),
        "user_id": id(user.as_uuid()),
        "role": role,
        "joined_at": micros(joined_at)?,
        "last_read_message_id": marker.map(|value| Bson::Binary(id(value.as_uuid()))).unwrap_or(Bson::Null),
    })
}
pub(super) async fn enabled(
    db: &mongodb::Database,
    session: &mut mongodb::ClientSession,
    user: UserId,
) -> Result<bool, mongodb::error::Error> {
    let key = id(user.as_uuid());
    let result = db
        .collection::<Document>("users")
        .update_one(
            doc! { "_id": &key, "disabled_at": Bson::Null },
            doc! { "$inc": { "_ww_revision": 1_i64 } },
        )
        .session(session)
        .await?;
    Ok(result.matched_count == 1)
}
async fn conversation_exists(
    db: &mongodb::Database,
    session: &mut mongodb::ClientSession,
    conversation: ConversationId,
) -> Result<bool, mongodb::error::Error> {
    let key = id(conversation.as_uuid());
    Ok(db
        .collection::<Document>("conversations")
        .update_one(
            doc! { "_id": &key },
            doc! { "$inc": { "_ww_revision": 1_i64 } },
        )
        .session(session)
        .await?
        .matched_count
        == 1)
}
async fn auth_role(
    db: &mongodb::Database,
    session: &mut mongodb::ClientSession,
    conversation: ConversationId,
    actor: UserId,
    owner_only: bool,
) -> Result<bool, mongodb::error::Error> {
    let filter = if owner_only {
        doc! { "conversation_id": id(conversation.as_uuid()), "user_id": id(actor.as_uuid()), "role": "owner" }
    } else {
        doc! { "conversation_id": id(conversation.as_uuid()), "user_id": id(actor.as_uuid()), "role": { "$in": ["owner", "admin"] } }
    };
    Ok(db
        .collection::<Document>("conversation_members")
        .find_one(filter)
        .session(session)
        .await?
        .is_some())
}

impl ConversationStore for MongoDbStorage {
    async fn member_ids(
        &self,
        conversation_id: ConversationId,
    ) -> Result<Vec<UserId>, ConversationStoreError> {
        use futures_util::TryStreamExt;
        let db = self.database();
        let key = id(conversation_id.as_uuid());
        let docs: Vec<Document> = db
            .collection("conversation_members")
            .find(doc! { "conversation_id": key })
            .await
            .map_err(db_error)?
            .try_collect()
            .await
            .map_err(db_error)?;
        docs.iter()
            .map(|member| {
                UserId::try_from(uuid(member, "user_id")?).map_err(|_| bad("invalid user ID"))
            })
            .collect()
    }
    async fn get_or_create_direct(
        &self,
        conversation: &Conversation,
        first: UserId,
        second: UserId,
    ) -> Result<StartDirectStoreOutcome, ConversationStoreError> {
        if conversation.kind() != ConversationKind::Direct {
            return Err(ConversationStoreError::InvalidConversationKind {
                expected: ConversationKind::Direct,
                actual: conversation.kind(),
            });
        }
        let db = self.database();
        let key = direct_key(first, second);
        let new_conversation = conv_doc(conversation, Some(key.clone()))?;
        let first_member = member_doc(
            conversation.id(),
            first,
            "direct",
            conversation.created_at(),
            None,
        )?;
        let mut direct_members = vec![first_member];
        if second != first {
            direct_members.push(member_doc(
                conversation.id(),
                second,
                "direct",
                conversation.created_at(),
                None,
            )?);
        }
        let lookup_db = db.clone();
        let result = transactions::run(&self.client, false, async move |session| {
            if let Some(existing) = db
                .collection::<Document>("conversations")
                .find_one(doc! { "direct_key": key.clone() })
                .session(&mut *session)
                .await?
            {
                return Ok((Some(existing), false, None));
            }
            let mut users = vec![first, second];
            users.sort();
            users.dedup();
            for user in users {
                if !enabled(&db, session, user).await? {
                    return Ok((None, false, Some(user)));
                }
            }
            db.collection::<Document>("conversations")
                .insert_one(new_conversation.clone())
                .session(&mut *session)
                .await?;
            db.collection::<Document>("conversation_members")
                .insert_many(direct_members.clone())
                .session(session)
                .await?;
            Ok((Some(new_conversation.clone()), true, None))
        })
        .await;
        match result {
            Ok((Some(document), created, None)) => {
                let persisted = read_conversation(&document)?;
                Ok(if created {
                    StartDirectStoreOutcome::Created(persisted)
                } else {
                    StartDirectStoreOutcome::Existing(persisted)
                })
            }
            Ok((None, false, Some(user))) => {
                Err(ConversationStoreError::ParticipantUnavailable(user))
            }
            Ok(_) => Err(bad("inconsistent direct-conversation transaction result")),
            Err(error) if matches!(error.kind.as_ref(), ErrorKind::Write(mongodb::error::WriteFailure::WriteError(write)) if write.code == 11000) =>
            {
                let document = lookup_db
                    .collection::<Document>("conversations")
                    .find_one(doc! { "direct_key": direct_key(first, second) })
                    .await
                    .map_err(db_error)?
                    .ok_or_else(|| bad("direct uniqueness race lost without winner"))?;
                Ok(StartDirectStoreOutcome::Existing(read_conversation(
                    &document,
                )?))
            }
            Err(error) => Err(db_error(error)),
        }
    }
    async fn create_group(
        &self,
        conversation: &Conversation,
        owner: UserId,
        participants: &[UserId],
    ) -> Result<(), ConversationStoreError> {
        if conversation.kind() != ConversationKind::Group {
            return Err(ConversationStoreError::InvalidConversationKind {
                expected: ConversationKind::Group,
                actual: conversation.kind(),
            });
        }
        let db = self.database();
        let conversation_doc = conv_doc(conversation, None)?;
        let mut users = participants.to_vec();
        users.push(owner);
        users.sort();
        users.dedup();
        let mut members = Vec::new();
        for user in &users {
            members.push(member_doc(
                conversation.id(),
                *user,
                if *user == owner { "owner" } else { "member" },
                conversation.created_at(),
                None,
            )?);
        }
        let result = transactions::run(&self.client, false, async move |session| {
            for user in &users {
                if !enabled(&db, session, *user).await? {
                    return Ok(Some(*user));
                }
            }
            db.collection::<Document>("conversations")
                .insert_one(conversation_doc.clone())
                .session(&mut *session)
                .await?;
            db.collection::<Document>("conversation_members")
                .insert_many(members.clone())
                .session(session)
                .await?;
            Ok(None)
        })
        .await
        .map_err(db_error)?;
        if let Some(user) = result {
            Err(ConversationStoreError::ParticipantUnavailable(user))
        } else {
            Ok(())
        }
    }
    async fn find_conversation(
        &self,
        id_value: ConversationId,
    ) -> Result<Option<Conversation>, ConversationStoreError> {
        self.database()
            .collection::<Document>("conversations")
            .find_one(doc! { "_id": id(id_value.as_uuid()) })
            .await
            .map_err(db_error)?
            .as_ref()
            .map(read_conversation)
            .transpose()
    }
    async fn find_member(
        &self,
        conversation: ConversationId,
        user: UserId,
    ) -> Result<Option<ConversationMember>, ConversationStoreError> {
        let db = self.database();
        let key = id(conversation.as_uuid());
        let user_key = id(user.as_uuid());
        let result = transactions::run(&self.client, true, async move |session| {
            let conversation = db
                .collection::<Document>("conversations")
                .find_one(doc! { "_id": &key })
                .session(&mut *session)
                .await?;
            let member = db
                .collection::<Document>("conversation_members")
                .find_one(doc! { "conversation_id": &key, "user_id": &user_key })
                .session(session)
                .await?;
            Ok((conversation, member))
        })
        .await
        .map_err(db_error)?;
        let Some((Some(conversation), Some(member))) = Some(result) else {
            return Ok(None);
        };
        let kind = conversation
            .get_str("kind")
            .map_err(|_| bad("invalid persisted conversation kind"))?;
        let member = read_member(&member)?;
        match (kind, member.role()) {
            ("direct", MembershipRole::Direct) | ("group", MembershipRole::Group(_)) => {
                Ok(Some(member))
            }
            _ => Err(bad("incompatible persisted membership role")),
        }
    }
    async fn add_group_member(
        &self,
        conversation: ConversationId,
        actor: UserId,
        member: UserId,
        now: OffsetDateTime,
    ) -> Result<AddMemberStoreOutcome, ConversationStoreError> {
        let at = normalized(now)?;
        let timestamp = micros(now)?;
        let db = self.database();
        let result = transactions::run(&self.client, false, async move |session| {
            if !conversation_exists(&db, session, conversation).await?
                || !auth_role(&db, session, conversation, actor, false).await?
            {
                return Ok(0_u8);
            }
            let key = id(conversation.as_uuid());
            let target = id(member.as_uuid());
            if db
                .collection::<Document>("conversation_members")
                .find_one(doc! { "conversation_id": &key, "user_id": &target })
                .session(&mut *session)
                .await?
                .is_some()
            {
                return Ok(1);
            }
            if !enabled(&db, session, member).await? {
                return Ok(2);
            }
            let latest = db
                .collection::<Document>("messages")
                .find_one(doc! { "conversation_id": &key })
                .sort(doc! { "_id": -1 })
                .session(&mut *session)
                .await?;
            let marker = latest.as_ref().and_then(|value| value.get("_id")).cloned();
            let mut doc = member_doc(conversation, member, "member", now, None)
                .map_err(|_| mongodb::error::Error::custom("invalid membership timestamp"))?;
            doc.insert("last_read_message_id", marker.unwrap_or(Bson::Null));
            db.collection::<Document>("conversation_members")
                .insert_one(doc)
                .session(&mut *session)
                .await?;
            db.collection::<Document>("conversations")
                .update_one(
                    doc! { "_id": &key },
                    doc! { "$set": { "updated_at": timestamp } },
                )
                .session(session)
                .await?;
            Ok(3)
        })
        .await
        .map_err(db_error)?;
        match result {
            0 => Ok(AddMemberStoreOutcome::NotAuthorized),
            1 => Ok(AddMemberStoreOutcome::AlreadyMember),
            2 => Err(ConversationStoreError::ParticipantUnavailable(member)),
            _ => Ok(AddMemberStoreOutcome::Added { updated_at: at }),
        }
    }
    async fn remove_group_member(
        &self,
        conversation: ConversationId,
        actor: UserId,
        member: UserId,
        now: OffsetDateTime,
    ) -> Result<ChangeStoreOutcome, ConversationStoreError> {
        let at = normalized(now)?;
        let timestamp = micros(now)?;
        let db = self.database();
        let changed = transactions::run(&self.client, false, async move |session| {
            if !conversation_exists(&db, session, conversation).await? || !auth_role(&db, session, conversation, actor, false).await? { return Ok(0_u8); }
            let result = db.collection::<Document>("conversation_members").delete_one(doc! { "conversation_id": id(conversation.as_uuid()), "user_id": id(member.as_uuid()), "role": { "$in": ["admin", "member"] } }).session(&mut *session).await?;
            if result.deleted_count > 0 { db.collection::<Document>("conversations").update_one(doc! { "_id": id(conversation.as_uuid()) }, doc! { "$set": { "updated_at": timestamp } }).session(session).await?; }
            Ok(if result.deleted_count > 0 { 1 } else { 2 })
        }).await.map_err(db_error)?;
        Ok(match changed {
            0 => ChangeStoreOutcome::NotAuthorized,
            1 => ChangeStoreOutcome::Changed { updated_at: at },
            _ => ChangeStoreOutcome::Unchanged,
        })
    }
    async fn set_group_member_role(
        &self,
        conversation: ConversationId,
        actor: UserId,
        member: UserId,
        role: GroupRole,
        now: OffsetDateTime,
    ) -> Result<ChangeStoreOutcome, ConversationStoreError> {
        let value = match role {
            GroupRole::Admin => "admin".to_owned(),
            GroupRole::Member => "member".to_owned(),
            GroupRole::Owner => return Err(ConversationStoreError::InvalidGroupRole),
        };
        let at = normalized(now)?;
        let timestamp = micros(now)?;
        let db = self.database();
        let db_for_transaction = db.clone();
        let changed = transactions::run(&self.client, false, async move |session| {
            let db = db_for_transaction.clone();
            let value = value.clone();
            if !conversation_exists(&db, session, conversation).await? || !auth_role(&db, session, conversation, actor, false).await? { return Ok(0_u8); }
            let filter = doc! { "conversation_id": id(conversation.as_uuid()), "user_id": id(member.as_uuid()), "role": { "$in": ["admin", "member"] } };
            let current = db.collection::<Document>("conversation_members").find_one(filter.clone()).session(&mut *session).await?;
            if current.as_ref().and_then(|doc| doc.get_str("role").ok()) == Some(value.as_str()) { return Ok(2); }
            let result = db.collection::<Document>("conversation_members").update_one(filter, doc! { "$set": { "role": value } }).session(&mut *session).await?;
            if result.modified_count > 0 { db.collection::<Document>("conversations").update_one(doc! { "_id": id(conversation.as_uuid()) }, doc! { "$set": { "updated_at": timestamp } }).session(session).await?; }
            Ok(if result.modified_count > 0 { 1 } else { 2 })
        }).await.map_err(db_error)?;
        Ok(match changed {
            0 => ChangeStoreOutcome::NotAuthorized,
            1 => ChangeStoreOutcome::Changed { updated_at: at },
            _ => ChangeStoreOutcome::Unchanged,
        })
    }
    async fn transfer_group_ownership(
        &self,
        conversation: ConversationId,
        actor: UserId,
        target: UserId,
        now: OffsetDateTime,
    ) -> Result<ChangeStoreOutcome, ConversationStoreError> {
        let at = normalized(now)?;
        let timestamp = micros(now)?;
        let db = self.database();
        let changed = transactions::run(&self.client, false, async move |session| {
            if !conversation_exists(&db, session, conversation).await? || !auth_role(&db, session, conversation, actor, true).await? { return Ok(0_u8); }
            if actor == target { return Ok(2); }
            let target_filter = doc! { "conversation_id": id(conversation.as_uuid()), "user_id": id(target.as_uuid()), "role": { "$in": ["admin", "member"] } };
            if db.collection::<Document>("conversation_members").find_one(target_filter.clone()).session(&mut *session).await?.is_none() { return Ok(2); }
            db.collection::<Document>("conversation_members").update_one(doc! { "conversation_id": id(conversation.as_uuid()), "user_id": id(actor.as_uuid()), "role": "owner" }, doc! { "$set": { "role": "admin" } }).session(&mut *session).await?;
            db.collection::<Document>("conversation_members").update_one(target_filter, doc! { "$set": { "role": "owner" } }).session(&mut *session).await?;
            db.collection::<Document>("conversations").update_one(doc! { "_id": id(conversation.as_uuid()) }, doc! { "$set": { "updated_at": timestamp } }).session(session).await?;
            Ok(1)
        }).await.map_err(db_error)?;
        Ok(match changed {
            0 => ChangeStoreOutcome::NotAuthorized,
            1 => ChangeStoreOutcome::Changed { updated_at: at },
            _ => ChangeStoreOutcome::Unchanged,
        })
    }
    async fn leave_group(
        &self,
        conversation: ConversationId,
        actor: UserId,
        now: OffsetDateTime,
    ) -> Result<LeaveStoreOutcome, ConversationStoreError> {
        let at = normalized(now)?;
        let timestamp = micros(now)?;
        let db = self.database();
        let result = transactions::run(&self.client, false, async move |session| {
            if !conversation_exists(&db, session, conversation).await? {
                return Ok(0_u8);
            }
            let key = id(conversation.as_uuid());
            let conv = db
                .collection::<Document>("conversations")
                .find_one(doc! { "_id": &key })
                .session(&mut *session)
                .await?;
            if conv.as_ref().and_then(|value| value.get_str("kind").ok()) == Some("direct") {
                return Ok(1);
            }
            let actor_key = id(actor.as_uuid());
            let member = db
                .collection::<Document>("conversation_members")
                .find_one(doc! { "conversation_id": &key, "user_id": &actor_key })
                .session(&mut *session)
                .await?;
            let Some(member) = member else {
                return Ok(2);
            };
            if member.get_str("role").ok() == Some("owner") {
                let count = db
                    .collection::<Document>("conversation_members")
                    .count_documents(doc! { "conversation_id": &key })
                    .session(&mut *session)
                    .await?;
                if count != 1 {
                    return Ok(3);
                }
                db.collection::<Document>("conversation_members")
                    .delete_many(doc! { "conversation_id": &key })
                    .session(&mut *session)
                    .await?;
                db.collection::<Document>("messages")
                    .delete_many(doc! { "conversation_id": &key })
                    .session(&mut *session)
                    .await?;
                db.collection::<Document>("conversations")
                    .delete_one(doc! { "_id": &key })
                    .session(session)
                    .await?;
                return Ok(4);
            }
            db.collection::<Document>("conversation_members")
                .delete_one(doc! { "conversation_id": &key, "user_id": &actor_key })
                .session(&mut *session)
                .await?;
            db.collection::<Document>("conversations")
                .update_one(
                    doc! { "_id": &key },
                    doc! { "$set": { "updated_at": timestamp } },
                )
                .session(session)
                .await?;
            Ok(5)
        })
        .await
        .map_err(db_error)?;
        Ok(match result {
            0 | 2 => LeaveStoreOutcome::NotMember,
            1 => LeaveStoreOutcome::Direct,
            3 => LeaveStoreOutcome::OwnershipTransferRequired,
            4 => LeaveStoreOutcome::ConversationDeleted,
            _ => LeaveStoreOutcome::Left { updated_at: at },
        })
    }
    async fn rename_group(
        &self,
        conversation: ConversationId,
        actor: UserId,
        title: &GroupTitle,
        now: OffsetDateTime,
    ) -> Result<ChangeStoreOutcome, ConversationStoreError> {
        let at = normalized(now)?;
        let timestamp = micros(now)?;
        let title = title.as_str().to_owned();
        let db = self.database();
        let result = transactions::run(&self.client, false, async move |session| {
            if !conversation_exists(&db, session, conversation).await?
                || !auth_role(&db, session, conversation, actor, false).await?
            {
                return Ok(0_u8);
            }
            let current = db
                .collection::<Document>("conversations")
                .find_one(doc! { "_id": id(conversation.as_uuid()), "kind": "group" })
                .session(&mut *session)
                .await?;
            let Some(current) = current else {
                return Ok(0);
            };
            if current.get_str("title").ok() == Some(title.as_str()) {
                return Ok(2);
            }
            db.collection::<Document>("conversations")
                .update_one(
                    doc! { "_id": id(conversation.as_uuid()), "kind": "group" },
                    doc! { "$set": { "title": &title, "updated_at": timestamp } },
                )
                .session(session)
                .await?;
            Ok(1)
        })
        .await
        .map_err(db_error)?;
        Ok(match result {
            0 => ChangeStoreOutcome::NotAuthorized,
            1 => ChangeStoreOutcome::Changed { updated_at: at },
            _ => ChangeStoreOutcome::Unchanged,
        })
    }
}
