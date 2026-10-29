use mongodb::{
    bson::{Binary, Bson, Document, doc, spec::BinarySubtype},
    error::ErrorKind,
};
use time::OffsetDateTime;

use super::{MongoDbStorage, conversations, messages, transactions};
use crate::{
    ConversationBoundary, ConversationId, ConversationMemberSummary,
    ConversationQueryOutcome as Outcome, ConversationSummary, MemberBoundary, MessagingQueryStore,
    PageLimit, QuerySlice, QueryStoreError, UserId, UserSummary, Username,
};

type Result<T> = mongodb::error::Result<T>;
fn invalid(message: &'static str) -> mongodb::error::Error {
    mongodb::error::Error::custom(message)
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
fn bin(id: uuid::Uuid) -> Binary {
    Binary {
        subtype: BinarySubtype::Uuid,
        bytes: id.as_bytes().to_vec(),
    }
}
fn uuid(document: &Document, field: &str) -> Result<uuid::Uuid> {
    let Some(Bson::Binary(value)) = document.get(field) else {
        return Err(invalid("persisted identifier is not BSON binary"));
    };
    if value.subtype != BinarySubtype::Uuid || value.bytes.len() != 16 {
        return Err(invalid("invalid persisted identifier"));
    }
    uuid::Uuid::from_slice(&value.bytes).map_err(|_| invalid("invalid persisted identifier"))
}
fn user(document: &Document) -> Result<UserSummary> {
    Ok(UserSummary {
        id: UserId::try_from(uuid(document, "_id")?).map_err(|_| invalid("invalid user ID"))?,
        username: document
            .get_str("username")
            .map_err(|_| invalid("invalid username"))?
            .parse::<Username>()
            .map_err(|_| invalid("invalid username"))?,
    })
}
fn micros(value: OffsetDateTime) -> Result<i64> {
    i64::try_from(value.unix_timestamp_nanos().div_euclid(1_000))
        .map_err(|_| invalid("timestamp out of range"))
}
fn embedded<'a>(document: &'a Document, field: &str) -> Result<&'a Document> {
    document
        .get_document(field)
        .map_err(|_| invalid("missing projection document"))
}
fn first<'a>(document: &'a Document, field: &str) -> Result<Option<&'a Document>> {
    document
        .get_array(field)
        .map_err(|_| invalid("missing projection array"))?
        .first()
        .map(|value| {
            value
                .as_document()
                .ok_or_else(|| invalid("invalid projection document"))
        })
        .transpose()
}
fn member_summary(document: &Document) -> Result<ConversationMemberSummary> {
    let member = conversations::read_member(document).map_err(|_| invalid("invalid membership"))?;
    let user = user(first(document, "user")?.ok_or_else(|| invalid("member user missing"))?)?;
    if user.id != member.user_id() {
        return Err(invalid("incompatible member user"));
    }
    Ok(ConversationMemberSummary {
        user,
        role: member.role(),
        joined_at: member.joined_at(),
    })
}

async fn authorized(
    db: &mongodb::Database,
    session: &mut mongodb::ClientSession,
    actor: UserId,
    conversation: ConversationId,
) -> Result<Outcome<()>> {
    let key = bin(conversation.as_uuid());
    if db
        .collection::<Document>("conversations")
        .find_one(doc! { "_id": &key })
        .session(&mut *session)
        .await?
        .is_none()
    {
        return Ok(Outcome::ConversationNotFound);
    }
    if db
        .collection::<Document>("conversation_members")
        .find_one(doc! { "conversation_id": &key, "user_id": bin(actor.as_uuid()) })
        .session(session)
        .await?
        .is_none()
    {
        return Ok(Outcome::NotMember);
    }
    Ok(Outcome::Found(()))
}

fn user_lookup(local_field: &str, output: &str) -> Document {
    doc! { "$lookup": { "from": "users", "localField": local_field, "foreignField": "_id", "as": output } }
}
fn lookahead(limit: PageLimit) -> i64 {
    limit.get() as i64 + 1
}

// Activity is derived, so the server must inspect each matching conversation's
// latest indexed message. Cursor/order/limit still execute before enrichment;
// only a page plus lookahead crosses the wire, with no per-row round trips.
fn inbox_pipeline(
    actor: UserId,
    before: Option<(i64, ConversationId)>,
    limit: i64,
    conversation: Option<ConversationId>,
) -> Vec<Document> {
    let mut filter = doc! { "user_id": bin(actor.as_uuid()) };
    if let Some(conversation) = conversation {
        filter.insert("conversation_id", bin(conversation.as_uuid()));
    }
    let mut pipeline = vec![
        doc! { "$match": filter },
        doc! { "$lookup": { "from": "conversations", "localField": "conversation_id", "foreignField": "_id", "as": "conversation" } },
        doc! { "$set": { "conversation": { "$first": "$conversation" } } },
        doc! { "$lookup": { "from": "messages", "let": { "conversation": "$conversation_id" }, "pipeline": [
            { "$match": { "$expr": { "$eq": ["$conversation_id", "$$conversation"] } } },
            { "$sort": { "_id": -1 } }, { "$limit": 1 }
        ], "as": "last" } },
        doc! { "$set": { "activity_at": { "$max": ["$conversation.updated_at", { "$first": "$last.created_at" }] } } },
    ];
    if let Some((at, id)) = before {
        pipeline.push(doc! { "$match": { "$or": [
            { "activity_at": { "$lt": at } },
            { "activity_at": at, "conversation_id": { "$lt": bin(id.as_uuid()) } }
        ] } });
    }
    pipeline.extend([
        doc! { "$sort": { "activity_at": -1, "conversation_id": -1 } },
        doc! { "$limit": limit },
        doc! { "$lookup": { "from": "messages", "let": { "conversation": "$conversation_id", "marker": "$last_read_message_id" }, "pipeline": [
            { "$match": { "sender_id": { "$ne": bin(actor.as_uuid()) }, "deleted_at": Bson::Null, "$expr": { "$and": [
                { "$eq": ["$conversation_id", "$$conversation"] },
                { "$or": [{ "$eq": ["$$marker", Bson::Null] }, { "$gt": ["$_id", "$$marker"] }] }
            ] } } }, { "$count": "count" }
        ], "as": "unread" } },
        // Prefer the other participant; a self-Direct falls back to the actor.
        doc! { "$lookup": { "from": "conversation_members", "let": { "conversation": "$conversation_id", "kind": "$conversation.kind" }, "pipeline": [
            { "$match": { "$expr": { "$and": [{ "$eq": ["$conversation_id", "$$conversation"] }, { "$eq": ["$$kind", "direct"] }] } } },
            { "$set": { "is_actor": { "$eq": ["$user_id", bin(actor.as_uuid())] } } },
            { "$sort": { "is_actor": 1, "user_id": 1 } }, { "$limit": 1 }
        ], "as": "peer" } },
        user_lookup("peer.user_id", "peer_user"),
    ]);
    pipeline
}

async fn summaries(
    db: &mongodb::Database,
    session: &mut mongodb::ClientSession,
    pipeline: Vec<Document>,
) -> Result<Vec<ConversationSummary>> {
    let mut cursor = db
        .collection::<Document>("conversation_members")
        .aggregate(pipeline)
        .session(&mut *session)
        .await?;
    let mut records = Vec::new();
    while let Some(record) = cursor.next(&mut *session).await.transpose()? {
        records.push(record);
    }
    let last_records = records
        .iter()
        .map(|record| first(record, "last").map(|last| last.cloned()))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect();
    let mut last_messages = messages::views(db, session, last_records)
        .await?
        .into_iter();
    records
        .into_iter()
        .map(|record| {
            let conversation = conversations::read_conversation(embedded(&record, "conversation")?)
                .map_err(|_| invalid("invalid conversation"))?;
            let membership =
                conversations::read_member(&record).map_err(|_| invalid("invalid membership"))?;
            let last_message = if first(&record, "last")?.is_some() {
                Some(
                    last_messages
                        .next()
                        .ok_or_else(|| invalid("last message missing"))?,
                )
            } else {
                None
            };
            let title = match conversation.title() {
                Some(title) => title.as_str().to_owned(),
                None => user(
                    first(&record, "peer_user")?.ok_or_else(|| invalid("direct peer missing"))?,
                )?
                .username
                .to_string(),
            };
            let unread_count = first(&record, "unread")?
                .map(|count| {
                    match count.get("count") {
                        Some(Bson::Int32(n)) => u64::try_from(*n),
                        Some(Bson::Int64(n)) => u64::try_from(*n),
                        _ => return Err(invalid("invalid unread count")),
                    }
                    .map_err(|_| invalid("invalid unread count"))
                })
                .transpose()?
                .unwrap_or(0);
            let activity_at = last_message
                .as_ref()
                .map(|m| m.created_at())
                .unwrap_or(conversation.updated_at())
                .max(conversation.updated_at());
            Ok(ConversationSummary {
                conversation,
                title,
                membership,
                unread_count,
                last_message,
                activity_at,
            })
        })
        .collect()
}

impl MessagingQueryStore for MongoDbStorage {
    async fn get_conversation_member(
        &self,
        actor: UserId,
        conversation: ConversationId,
        target: UserId,
    ) -> std::result::Result<Outcome<Option<ConversationMemberSummary>>, QueryStoreError> {
        let db = self.database();
        transactions::run(&self.client, true, async move |session| {
            match authorized(&db, session, actor, conversation).await? {
                Outcome::ConversationNotFound => return Ok(Outcome::ConversationNotFound),
                Outcome::NotMember => return Ok(Outcome::NotMember),
                Outcome::Found(()) => (),
            }
            let mut cursor = db.collection::<Document>("conversation_members").aggregate(vec![
                doc! { "$match": { "conversation_id": bin(conversation.as_uuid()), "user_id": bin(target.as_uuid()) } },
                user_lookup("user_id", "user"),
            ]).session(&mut *session).await?;
            Ok(Outcome::Found(cursor.next(&mut *session).await.transpose()?.as_ref().map(member_summary).transpose()?))
        }).await.map_err(db_error)
    }
    async fn find_users(
        &self,
        name: &Username,
    ) -> std::result::Result<Vec<UserSummary>, QueryStoreError> {
        self.database()
            .collection::<Document>("users")
            .find_one(doc! { "username": name.as_str(), "disabled_at": Bson::Null })
            .await
            .map_err(db_error)?
            .as_ref()
            .map(user)
            .transpose()
            .map(|user| user.into_iter().collect())
            .map_err(db_error)
    }
    async fn list_conversations(
        &self,
        actor: UserId,
        before: Option<ConversationBoundary>,
        limit: PageLimit,
    ) -> std::result::Result<QuerySlice<ConversationSummary>, QueryStoreError> {
        let db = self.database();
        let before = before
            .map(|b| micros(b.activity_at).map(|at| (at, b.conversation_id)))
            .transpose()
            .map_err(db_error)?;
        let mut items = transactions::run(&self.client, true, async move |session| {
            summaries(
                &db,
                session,
                inbox_pipeline(actor, before, lookahead(limit), None),
            )
            .await
        })
        .await
        .map_err(db_error)?;
        let has_more = items.len() > limit.get();
        items.truncate(limit.get());
        Ok(QuerySlice { items, has_more })
    }
    async fn get_conversation_summary(
        &self,
        actor: UserId,
        conversation: ConversationId,
    ) -> std::result::Result<Outcome<ConversationSummary>, QueryStoreError> {
        let db = self.database();
        transactions::run(&self.client, true, async move |session| {
            match authorized(&db, session, actor, conversation).await? {
                Outcome::ConversationNotFound => return Ok(Outcome::ConversationNotFound),
                Outcome::NotMember => return Ok(Outcome::NotMember),
                Outcome::Found(()) => (),
            }
            Ok(Outcome::Found(
                summaries(
                    &db,
                    session,
                    inbox_pipeline(actor, None, 1, Some(conversation)),
                )
                .await?
                .pop()
                .ok_or_else(|| invalid("summary missing"))?,
            ))
        })
        .await
        .map_err(db_error)
    }
    async fn list_conversation_members(
        &self,
        actor: UserId,
        conversation: ConversationId,
        after: Option<MemberBoundary>,
        limit: PageLimit,
    ) -> std::result::Result<Outcome<QuerySlice<ConversationMemberSummary>>, QueryStoreError> {
        let db = self.database();
        let mut filter = doc! { "conversation_id": bin(conversation.as_uuid()) };
        if let Some(boundary) = after {
            let at = micros(boundary.joined_at).map_err(db_error)?;
            filter.insert(
                "$or",
                vec![
                    doc! { "joined_at": { "$gt": at } },
                    doc! { "joined_at": at, "user_id": { "$gt": bin(boundary.user_id.as_uuid()) } },
                ],
            );
        }
        transactions::run(&self.client, true, async move |session| {
            match authorized(&db, session, actor, conversation).await? {
                Outcome::ConversationNotFound => return Ok(Outcome::ConversationNotFound),
                Outcome::NotMember => return Ok(Outcome::NotMember),
                Outcome::Found(()) => (),
            }
            let mut cursor = db
                .collection::<Document>("conversation_members")
                .aggregate(vec![
                    doc! { "$match": filter.clone() },
                    doc! { "$sort": { "joined_at": 1, "user_id": 1 } },
                    doc! { "$limit": lookahead(limit) },
                    user_lookup("user_id", "user"),
                ])
                .session(&mut *session)
                .await?;
            let mut items = Vec::new();
            while let Some(record) = cursor.next(&mut *session).await.transpose()? {
                items.push(member_summary(&record)?);
            }
            let has_more = items.len() > limit.get();
            items.truncate(limit.get());
            Ok(Outcome::Found(QuerySlice { items, has_more }))
        })
        .await
        .map_err(db_error)
    }
}
