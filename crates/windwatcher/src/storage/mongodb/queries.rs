use std::{error::Error, fmt};

use mongodb::{
    bson::{Bson, Document, doc, spec::BinarySubtype},
    error::ErrorKind,
};
use time::OffsetDateTime;

use super::{MongoDbStorage, conversations, messages, transactions};
use crate::{
    ConversationBoundary, ConversationId, ConversationMemberSummary,
    ConversationQueryOutcome as Outcome, ConversationSummary, MemberBoundary, MessageId,
    MessagingQueryStore, PageLimit, QuerySlice, QueryStoreError, UserId, UserSummary, Username,
};

#[derive(Debug)]
struct Invalid(&'static str);
impl fmt::Display for Invalid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl Error for Invalid {}
fn bad(message: &'static str) -> QueryStoreError {
    QueryStoreError::Unexpected(Box::new(Invalid(message)))
}
fn db_error(error: mongodb::error::Error) -> QueryStoreError {
    if error.contains_label("TransientTransactionError")
        || error.contains_label("UnknownTransactionCommitResult")
        || matches!(
            error.kind.as_ref(),
            ErrorKind::Io(_)
                | ErrorKind::ConnectionPoolCleared { .. }
                | ErrorKind::ServerSelection { .. }
        )
    {
        QueryStoreError::Unavailable(Box::new(error))
    } else {
        QueryStoreError::Unexpected(Box::new(error))
    }
}
fn bin(id: uuid::Uuid) -> mongodb::bson::Binary {
    mongodb::bson::Binary {
        subtype: BinarySubtype::Uuid,
        bytes: id.as_bytes().to_vec(),
    }
}
fn uuid(document: &Document, key: &str) -> Result<uuid::Uuid, QueryStoreError> {
    let Some(Bson::Binary(value)) = document.get(key) else {
        return Err(bad("persisted query identifier is not BSON binary"));
    };
    if value.subtype != BinarySubtype::Uuid || value.bytes.len() != 16 {
        return Err(bad("invalid persisted query identifier"));
    }
    uuid::Uuid::from_slice(&value.bytes).map_err(|_| bad("invalid persisted query identifier"))
}
fn username(document: &Document, key: &str) -> Result<Username, QueryStoreError> {
    document
        .get_str(key)
        .map_err(|_| bad("persisted username is not a string"))?
        .parse()
        .map_err(|_| bad("invalid persisted username"))
}
fn marker(member: &Document) -> Result<Option<MessageId>, QueryStoreError> {
    match member.get("last_read_message_id") {
        Some(Bson::Null) => Ok(None),
        Some(Bson::Binary(value))
            if value.subtype == BinarySubtype::Uuid && value.bytes.len() == 16 =>
        {
            Ok(Some(
                MessageId::try_from(
                    uuid::Uuid::from_slice(&value.bytes).map_err(|_| bad("invalid read marker"))?,
                )
                .map_err(|_| bad("invalid read marker"))?,
            ))
        }
        _ => Err(bad("invalid persisted read marker")),
    }
}
fn micros(value: OffsetDateTime) -> Result<i64, QueryStoreError> {
    i64::try_from(value.unix_timestamp_nanos().div_euclid(1_000))
        .map_err(|_| bad("timestamp out of range"))
}
async fn authorized(
    db: &mongodb::Database,
    session: &mut mongodb::ClientSession,
    actor: UserId,
    conversation: ConversationId,
) -> mongodb::error::Result<Option<Option<Document>>> {
    let key = bin(conversation.as_uuid());
    if db
        .collection::<Document>("conversations")
        .find_one(doc! { "_id": &key })
        .session(&mut *session)
        .await?
        .is_none()
    {
        return Ok(None);
    }
    let member = db
        .collection::<Document>("conversation_members")
        .find_one(doc! { "conversation_id": &key, "user_id": bin(actor.as_uuid()) })
        .session(session)
        .await?;
    Ok(Some(member))
}
async fn summary(
    db: &mongodb::Database,
    session: &mut mongodb::ClientSession,
    actor: UserId,
    conversation: Document,
    membership: Document,
) -> mongodb::error::Result<ConversationSummary> {
    let conv = conversations::read_conversation(&conversation)
        .map_err(|_| mongodb::error::Error::custom("invalid conversation"))?;
    let member = conversations::read_member(&membership)
        .map_err(|_| mongodb::error::Error::custom("invalid membership"))?;
    let key = bin(conv.id().as_uuid());
    let mut cursor = db
        .collection::<Document>("messages")
        .find(doc! { "conversation_id": &key })
        .sort(doc! { "_id": -1 })
        .limit(1)
        .session(&mut *session)
        .await?;
    let last_record = cursor.next(&mut *session).await.transpose()?;
    let last_message = match last_record {
        Some(record) => Some(messages::view(db, session, record).await?),
        None => None,
    };
    let unread_filter = {
        let mut filter = doc! { "conversation_id": &key, "sender_id": { "$ne": bin(actor.as_uuid()) }, "deleted_at": Bson::Null };
        if let Some(marker) =
            marker(&membership).map_err(|_| mongodb::error::Error::custom("invalid read marker"))?
        {
            filter.insert("_id", doc! { "$gt": bin(marker.as_uuid()) });
        }
        filter
    };
    let unread_count = db
        .collection::<Document>("messages")
        .count_documents(unread_filter)
        .session(&mut *session)
        .await?;
    let title = match conv.title() {
        Some(title) => title.as_str().to_owned(),
        None => {
            let peer = db
                .collection::<Document>("conversation_members")
                .find_one(
                    doc! { "conversation_id": &key, "user_id": { "$ne": bin(actor.as_uuid()) } },
                )
                .session(&mut *session)
                .await?;
            let peer = match peer {
                Some(peer) => peer,
                None => db
                    .collection::<Document>("conversation_members")
                    .find_one(doc! { "conversation_id": &key, "user_id": bin(actor.as_uuid()) })
                    .session(&mut *session)
                    .await?
                    .ok_or_else(|| mongodb::error::Error::custom("direct participant missing"))?,
            };
            let peer_id = UserId::try_from(
                uuid(&peer, "user_id")
                    .map_err(|_| mongodb::error::Error::custom("invalid peer ID"))?,
            )
            .map_err(|_| mongodb::error::Error::custom("invalid peer ID"))?;
            let user = db
                .collection::<Document>("users")
                .find_one(doc! { "_id": bin(peer_id.as_uuid()) })
                .session(session)
                .await?
                .ok_or_else(|| mongodb::error::Error::custom("direct peer user missing"))?;
            username(&user, "username")
                .map_err(|_| mongodb::error::Error::custom("invalid peer username"))?
                .to_string()
        }
    };
    let activity = last_message
        .as_ref()
        .map(|m| m.created_at())
        .unwrap_or(conv.updated_at())
        .max(conv.updated_at());
    Ok(ConversationSummary {
        conversation: conv,
        title,
        membership: member,
        unread_count,
        last_message,
        activity_at: activity,
    })
}
async fn authorized_member_summary(
    db: &mongodb::Database,
    session: &mut mongodb::ClientSession,
    conversation: ConversationId,
    target: UserId,
) -> mongodb::error::Result<Option<ConversationMemberSummary>> {
    let member = db.collection::<Document>("conversation_members").find_one(doc! { "conversation_id": bin(conversation.as_uuid()), "user_id": bin(target.as_uuid()) }).session(&mut *session).await?;
    let Some(member) = member else {
        return Ok(None);
    };
    let user = db
        .collection::<Document>("users")
        .find_one(doc! { "_id": bin(target.as_uuid()) })
        .session(&mut *session)
        .await?
        .ok_or_else(|| mongodb::error::Error::custom("member user missing"))?;
    let member = conversations::read_member(&member)
        .map_err(|_| mongodb::error::Error::custom("invalid membership"))?;
    Ok(Some(ConversationMemberSummary {
        user: UserSummary {
            id: target,
            username: username(&user, "username")
                .map_err(|_| mongodb::error::Error::custom("invalid username"))?,
        },
        role: member.role(),
        joined_at: member.joined_at(),
    }))
}

impl MessagingQueryStore for MongoDbStorage {
    async fn get_conversation_member(
        &self,
        actor: UserId,
        conversation: ConversationId,
        target: UserId,
    ) -> Result<Outcome<Option<ConversationMemberSummary>>, QueryStoreError> {
        let db = self.database();
        let result = transactions::run(&self.client, true, async move |session| {
            match authorized(&db, session, actor, conversation).await? {
                None => return Ok((0_u8, None)),
                Some(None) => return Ok((1, None)),
                Some(Some(_)) => (),
            }
            Ok((
                2,
                authorized_member_summary(&db, session, conversation, target).await?,
            ))
        })
        .await
        .map_err(db_error)?;
        Ok(match result {
            (0, _) => Outcome::ConversationNotFound,
            (1, _) => Outcome::NotMember,
            (_, member) => Outcome::Found(member),
        })
    }
    async fn find_users(&self, name: &Username) -> Result<Vec<UserSummary>, QueryStoreError> {
        let doc = self
            .database()
            .collection::<Document>("users")
            .find_one(doc! { "username": name.as_str(), "disabled_at": Bson::Null })
            .await
            .map_err(db_error)?;
        let Some(doc) = doc else {
            return Ok(Vec::new());
        };
        Ok(vec![UserSummary {
            id: UserId::try_from(uuid(&doc, "_id")?).map_err(|_| bad("invalid user ID"))?,
            username: username(&doc, "username")?,
        }])
    }
    async fn list_conversations(
        &self,
        actor: UserId,
        before: Option<ConversationBoundary>,
        limit: PageLimit,
    ) -> Result<QuerySlice<ConversationSummary>, QueryStoreError> {
        let db = self.database();
        let result = transactions::run(&self.client, true, async move |session| {
            let mut cursor = db
                .collection::<Document>("conversation_members")
                .find(doc! { "user_id": bin(actor.as_uuid()) })
                .session(&mut *session)
                .await?;
            let mut summaries = Vec::new();
            while let Some(member) = cursor.next(&mut *session).await.transpose()? {
                let conversation_id = uuid(&member, "conversation_id")
                    .map_err(|_| mongodb::error::Error::custom("invalid conversation ID"))?;
                let conv = db
                    .collection::<Document>("conversations")
                    .find_one(doc! { "_id": bin(conversation_id) })
                    .session(&mut *session)
                    .await?
                    .ok_or_else(|| {
                        mongodb::error::Error::custom("membership conversation missing")
                    })?;
                summaries.push(summary(&db, session, actor, conv, member).await?);
            }
            Ok(summaries)
        })
        .await
        .map_err(db_error)?;
        let mut items: Vec<_> = result
            .into_iter()
            .filter(|summary| {
                before.is_none_or(|b| {
                    summary.activity_at < b.activity_at
                        || (summary.activity_at == b.activity_at
                            && summary.conversation.id() < b.conversation_id)
                })
            })
            .collect();
        items.sort_by(|a, b| {
            b.activity_at
                .cmp(&a.activity_at)
                .then_with(|| b.conversation.id().cmp(&a.conversation.id()))
        });
        let has_more = items.len() > limit.get();
        items.truncate(limit.get());
        Ok(QuerySlice { items, has_more })
    }
    async fn get_conversation_summary(
        &self,
        actor: UserId,
        conversation: ConversationId,
    ) -> Result<Outcome<ConversationSummary>, QueryStoreError> {
        let db = self.database();
        let result = transactions::run(&self.client, true, async move |session| {
            let Some(conv) = db.collection::<Document>("conversations").find_one(doc! { "_id": bin(conversation.as_uuid()) }).session(&mut *session).await? else { return Ok((0_u8, None)); };
            let member = db.collection::<Document>("conversation_members").find_one(doc! { "conversation_id": bin(conversation.as_uuid()), "user_id": bin(actor.as_uuid()) }).session(&mut *session).await?;
            let Some(member) = member else { return Ok((1, None)); };
            Ok((2, Some(summary(&db, session, actor, conv, member).await?)))
        }).await.map_err(db_error)?;
        Ok(match result {
            (0, _) => Outcome::ConversationNotFound,
            (1, _) => Outcome::NotMember,
            (_, Some(value)) => Outcome::Found(value),
            _ => return Err(bad("summary result missing")),
        })
    }
    async fn list_conversation_members(
        &self,
        actor: UserId,
        conversation: ConversationId,
        after: Option<MemberBoundary>,
        limit: PageLimit,
    ) -> Result<Outcome<QuerySlice<ConversationMemberSummary>>, QueryStoreError> {
        let db = self.database();
        let result = transactions::run(&self.client, true, async move |session| {
            let key = bin(conversation.as_uuid());
            if db
                .collection::<Document>("conversations")
                .find_one(doc! { "_id": &key })
                .session(&mut *session)
                .await?
                .is_none()
            {
                return Ok((0_u8, Vec::new()));
            }
            if db
                .collection::<Document>("conversation_members")
                .find_one(doc! { "conversation_id": &key, "user_id": bin(actor.as_uuid()) })
                .session(&mut *session)
                .await?
                .is_none()
            {
                return Ok((1, Vec::new()));
            }
            let mut cursor = db
                .collection::<Document>("conversation_members")
                .find(doc! { "conversation_id": &key })
                .session(&mut *session)
                .await?;
            let mut values = Vec::new();
            while let Some(member) = cursor.next(&mut *session).await.transpose()? {
                let user = UserId::try_from(
                    uuid(&member, "user_id")
                        .map_err(|_| mongodb::error::Error::custom("invalid user ID"))?,
                )
                .map_err(|_| mongodb::error::Error::custom("invalid user ID"))?;
                if let Some(summary) =
                    authorized_member_summary(&db, session, conversation, user).await?
                {
                    values.push(summary);
                }
            }
            Ok((2, values))
        })
        .await
        .map_err(db_error)?;
        if result.0 == 0 {
            return Ok(Outcome::ConversationNotFound);
        }
        if result.0 == 1 {
            return Ok(Outcome::NotMember);
        }
        let boundary = after
            .map(|b| micros(b.joined_at).map(|t| (t, b.user_id)))
            .transpose()?;
        let mut items = result
            .1
            .into_iter()
            .filter(|item| {
                boundary.is_none_or(|(at, user)| {
                    item.joined_at.unix_timestamp_nanos().div_euclid(1000) > i128::from(at)
                        || (item.joined_at.unix_timestamp_nanos().div_euclid(1000)
                            == i128::from(at)
                            && item.user.id > user)
                })
            })
            .collect::<Vec<_>>();
        items.sort_by(|a, b| {
            a.joined_at
                .cmp(&b.joined_at)
                .then_with(|| a.user.id.cmp(&b.user.id))
        });
        let has_more = items.len() > limit.get();
        items.truncate(limit.get());
        Ok(Outcome::Found(QuerySlice { items, has_more }))
    }
}
