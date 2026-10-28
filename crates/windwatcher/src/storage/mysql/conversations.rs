use super::{MySqlStorage, date, micros, raw, transient};
use crate::{
    AddMemberStoreOutcome, ChangeStoreOutcome, Conversation, ConversationId, ConversationKind,
    ConversationMember, ConversationStore, ConversationStoreError, GroupRole, GroupTitle,
    LeaveStoreOutcome, MembershipRole, StartDirectStoreOutcome, UserId,
};
use sqlx::{MySqlConnection, Row};
use std::{error::Error, fmt};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug)]
struct Bad(&'static str);
impl fmt::Display for Bad {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl Error for Bad {}
fn bad(s: &'static str) -> ConversationStoreError {
    ConversationStoreError::Unexpected(Box::new(Bad(s)))
}
fn db(e: sqlx::Error) -> ConversationStoreError {
    if transient(&e) {
        ConversationStoreError::Unavailable(Box::new(e))
    } else if matches!(&e,sqlx::Error::Database(d)if d.is_unique_violation()||d.is_foreign_key_violation()||d.is_check_violation())
    {
        ConversationStoreError::Conflict
    } else {
        ConversationStoreError::Unexpected(Box::new(e))
    }
}
fn normalized(t: OffsetDateTime) -> Result<OffsetDateTime, ConversationStoreError> {
    date(micros(t).map_err(bad)?).map_err(bad)
}
fn uuid(bytes: Vec<u8>) -> Result<Uuid, ConversationStoreError> {
    Uuid::from_slice(&bytes).map_err(|_| bad("invalid persisted identifier"))
}
pub(super) fn read_conversation(
    r: &sqlx::mysql::MySqlRow,
) -> Result<Conversation, ConversationStoreError> {
    let id = ConversationId::try_from(uuid(r.try_get("id").map_err(db)?)?)
        .map_err(|_| bad("invalid conversation ID"))?;
    let kind = match r.try_get::<String, _>("kind").map_err(db)?.as_str() {
        "direct" => ConversationKind::Direct,
        "group" => ConversationKind::Group,
        _ => return Err(bad("invalid conversation kind")),
    };
    let title: Option<Vec<u8>> = r.try_get("title").map_err(db)?;
    let title = title
        .map(|v| {
            String::from_utf8(v)
                .map_err(|_| bad("invalid title encoding"))
                .and_then(|v| GroupTitle::new(v).map_err(|_| bad("invalid title")))
        })
        .transpose()?;
    let created = date(r.try_get("created_at").map_err(db)?).map_err(bad)?;
    let updated = date(r.try_get("updated_at").map_err(db)?).map_err(bad)?;
    Conversation::restore(id, kind, title, created, updated)
        .map_err(|_| bad("incoherent conversation"))
}
pub(super) fn read_member(
    r: &sqlx::mysql::MySqlRow,
) -> Result<ConversationMember, ConversationStoreError> {
    let c = ConversationId::try_from(uuid(r.try_get("conversation_id").map_err(db)?)?)
        .map_err(|_| bad("invalid conversation ID"))?;
    let u = UserId::try_from(uuid(r.try_get("user_id").map_err(db)?)?)
        .map_err(|_| bad("invalid user ID"))?;
    let role = match r.try_get::<String, _>("role").map_err(db)?.as_str() {
        "direct" => MembershipRole::Direct,
        "owner" => MembershipRole::Group(GroupRole::Owner),
        "admin" => MembershipRole::Group(GroupRole::Admin),
        "member" => MembershipRole::Group(GroupRole::Member),
        _ => return Err(bad("invalid membership role")),
    };
    let joined = date(r.try_get("joined_at").map_err(db)?).map_err(bad)?;
    let marker: Option<Vec<u8>> = r.try_get("last_read_message_id").map_err(db)?;
    let marker = marker
        .map(|v| {
            uuid(v)
                .and_then(|v| crate::MessageId::try_from(v).map_err(|_| bad("invalid read marker")))
        })
        .transpose()?;
    Ok(ConversationMember::restore(c, u, role, joined, marker))
}
async fn lock(c: &mut MySqlConnection, id: ConversationId) -> Result<bool, ConversationStoreError> {
    Ok(
        sqlx::query("SELECT id FROM conversations WHERE id=? FOR UPDATE")
            .bind(raw(id))
            .fetch_optional(c)
            .await
            .map_err(db)?
            .is_some(),
    )
}
async fn enabled(c: &mut MySqlConnection, id: UserId) -> Result<(), ConversationStoreError> {
    let row = sqlx::query("SELECT disabled_at FROM users WHERE id=? FOR UPDATE")
        .bind(raw(id))
        .fetch_optional(c)
        .await
        .map_err(db)?;
    if row.is_some_and(|r| r.try_get::<Option<i64>, _>("disabled_at").ok() == Some(None)) {
        Ok(())
    } else {
        Err(ConversationStoreError::ParticipantUnavailable(id))
    }
}
pub(super) async fn create_group(
    c: &mut MySqlConnection,
    conv: &Conversation,
    owner: UserId,
    participants: &[UserId],
) -> Result<(), ConversationStoreError> {
    if conv.kind() != ConversationKind::Group {
        return Err(ConversationStoreError::InvalidConversationKind {
            expected: ConversationKind::Group,
            actual: conv.kind(),
        });
    }
    let title = conv
        .title()
        .ok_or(ConversationStoreError::InvalidConversationKind {
            expected: ConversationKind::Group,
            actual: ConversationKind::Direct,
        })?;
    let mut users = participants.to_vec();
    users.push(owner);
    users.sort();
    users.dedup();
    for u in &users {
        enabled(c, *u).await?
    }
    let t = micros(conv.created_at()).map_err(bad)?;
    sqlx::query("INSERT INTO conversations(id,kind,title,direct_key,created_at,updated_at) VALUES(?,'group',?,NULL,?,?)").bind(raw(conv.id())).bind(title.as_str().as_bytes()).bind(t).bind(micros(conv.updated_at()).map_err(bad)?).execute(&mut *c).await.map_err(db)?;
    sqlx::query("INSERT INTO conversation_members(conversation_id,user_id,role,joined_at) VALUES(?,?,'owner',?)").bind(raw(conv.id())).bind(raw(owner)).bind(t).execute(&mut *c).await.map_err(db)?;
    for u in users {
        if u != owner {
            sqlx::query("INSERT INTO conversation_members(conversation_id,user_id,role,joined_at) VALUES(?,?,'member',?)").bind(raw(conv.id())).bind(raw(u)).bind(t).execute(&mut *c).await.map_err(db)?;
        }
    }
    Ok(())
}
pub(super) async fn create_direct(
    tx: &mut MySqlConnection,
    conv: &Conversation,
    a: UserId,
    b: UserId,
) -> Result<StartDirectStoreOutcome, ConversationStoreError> {
    if conv.kind() != ConversationKind::Direct {
        return Err(ConversationStoreError::InvalidConversationKind {
            expected: ConversationKind::Direct,
            actual: conv.kind(),
        });
    }
    let mut pair = [a.as_uuid().into_bytes(), b.as_uuid().into_bytes()];
    pair.sort();
    let direct_key = [pair[0].as_slice(), pair[1].as_slice()].concat();
    if let Some(row) = sqlx::query(
        "SELECT id,kind,title,created_at,updated_at FROM conversations WHERE direct_key=?",
    )
    .bind(&direct_key)
    .fetch_optional(&mut *tx)
    .await
    .map_err(db)?
    {
        return Ok(StartDirectStoreOutcome::Existing(read_conversation(&row)?));
    }
    let mut ids = vec![a, b];
    ids.sort();
    ids.dedup();
    for id in ids {
        enabled(&mut *tx, id).await?;
    }
    let created = micros(conv.created_at()).map_err(bad)?;
    let result=sqlx::query("INSERT INTO conversations(id,kind,title,direct_key,created_at,updated_at) VALUES(?,'direct',NULL,?,?,?)")
        .bind(raw(conv.id())).bind(&direct_key).bind(created).bind(micros(conv.updated_at()).map_err(bad)?)
        .execute(&mut *tx).await;
    if let Err(error) = result {
        if !matches!(&error, sqlx::Error::Database(e) if e.is_unique_violation()) {
            return Err(db(error));
        }
        let row=sqlx::query("SELECT id,kind,title,created_at,updated_at FROM conversations WHERE direct_key=? FOR UPDATE")
            .bind(&direct_key).fetch_optional(&mut *tx).await.map_err(db)?;
        return match row {
            Some(row) => Ok(StartDirectStoreOutcome::Existing(read_conversation(&row)?)),
            None => Err(db(error)),
        };
    }
    let mut members = vec![a, b];
    members.sort();
    members.dedup();
    for id in members {
        sqlx::query("INSERT INTO conversation_members(conversation_id,user_id,role,joined_at) VALUES(?,?,'direct',?)")
            .bind(raw(conv.id())).bind(raw(id)).bind(created).execute(&mut *tx).await.map_err(db)?;
    }
    Ok(StartDirectStoreOutcome::Created(conv.clone()))
}

impl ConversationStore for MySqlStorage {
    async fn member_ids(&self, id: ConversationId) -> Result<Vec<UserId>, ConversationStoreError> {
        let rows = sqlx::query("SELECT user_id FROM conversation_members WHERE conversation_id=?")
            .bind(raw(id))
            .fetch_all(&self.pool)
            .await
            .map_err(db)?;
        rows.into_iter()
            .map(|r| {
                UserId::try_from(
                    uuid(r.try_get("user_id").map_err(db)?).map_err(|_| bad("invalid user ID"))?,
                )
                .map_err(|_| bad("invalid user ID"))
            })
            .collect()
    }
    async fn get_or_create_direct(
        &self,
        conv: &Conversation,
        a: UserId,
        b: UserId,
    ) -> Result<StartDirectStoreOutcome, ConversationStoreError> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        let result = create_direct(&mut tx, conv, a, b).await?;
        tx.commit().await.map_err(db)?;
        Ok(result)
    }
    async fn create_group(
        &self,
        c: &Conversation,
        o: UserId,
        p: &[UserId],
    ) -> Result<(), ConversationStoreError> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        create_group(&mut tx, c, o, p).await?;
        tx.commit().await.map_err(db)
    }
    async fn find_conversation(
        &self,
        id: ConversationId,
    ) -> Result<Option<Conversation>, ConversationStoreError> {
        sqlx::query("SELECT id,kind,title,created_at,updated_at FROM conversations WHERE id=?")
            .bind(raw(id))
            .fetch_optional(&self.pool)
            .await
            .map_err(db)?
            .map(|r| read_conversation(&r))
            .transpose()
    }
    async fn find_member(
        &self,
        c: ConversationId,
        u: UserId,
    ) -> Result<Option<ConversationMember>, ConversationStoreError> {
        sqlx::query("SELECT m.conversation_id,m.user_id,m.role,m.joined_at,m.last_read_message_id FROM conversation_members m WHERE m.conversation_id=? AND m.user_id=?").bind(raw(c)).bind(raw(u)).fetch_optional(&self.pool).await.map_err(db)?.map(|r|read_member(&r)).transpose()
    }
    async fn add_group_member(
        &self,
        id: ConversationId,
        actor: UserId,
        member: UserId,
        now: OffsetDateTime,
    ) -> Result<AddMemberStoreOutcome, ConversationStoreError> {
        let at = normalized(now)?;
        let mut tx = self.pool.begin().await.map_err(db)?;
        if !lock(&mut tx, id).await? {
            return Ok(AddMemberStoreOutcome::NotAuthorized);
        }
        let auth:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM conversation_members WHERE conversation_id=? AND user_id=? AND role IN ('owner','admin'))").bind(raw(id)).bind(raw(actor)).fetch_one(&mut *tx).await.map_err(db)?;
        if !auth {
            return Ok(AddMemberStoreOutcome::NotAuthorized);
        }
        let exists:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM conversation_members WHERE conversation_id=? AND user_id=?)").bind(raw(id)).bind(raw(member)).fetch_one(&mut *tx).await.map_err(db)?;
        if exists {
            tx.commit().await.map_err(db)?;
            return Ok(AddMemberStoreOutcome::AlreadyMember);
        }
        enabled(&mut tx, member).await?;
        let latest: Option<Vec<u8>> = sqlx::query_scalar(
            "SELECT id FROM messages WHERE conversation_id=? ORDER BY id DESC LIMIT 1",
        )
        .bind(raw(id))
        .fetch_optional(&mut *tx)
        .await
        .map_err(db)?;
        sqlx::query("INSERT INTO conversation_members(conversation_id,user_id,role,joined_at,last_read_message_id) VALUES(?,?,'member',?,?)").bind(raw(id)).bind(raw(member)).bind(micros(now).map_err(bad)?).bind(latest).execute(&mut *tx).await.map_err(db)?;
        sqlx::query("UPDATE conversations SET updated_at=? WHERE id=?")
            .bind(micros(now).map_err(bad)?)
            .bind(raw(id))
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        tx.commit().await.map_err(db)?;
        Ok(AddMemberStoreOutcome::Added { updated_at: at })
    }
    async fn remove_group_member(
        &self,
        id: ConversationId,
        actor: UserId,
        member: UserId,
        now: OffsetDateTime,
    ) -> Result<ChangeStoreOutcome, ConversationStoreError> {
        let at = normalized(now)?;
        let mut tx = self.pool.begin().await.map_err(db)?;
        if !lock(&mut tx, id).await? {
            return Ok(ChangeStoreOutcome::NotAuthorized);
        }
        let auth:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM conversation_members WHERE conversation_id=? AND user_id=? AND role IN ('owner','admin'))").bind(raw(id)).bind(raw(actor)).fetch_one(&mut *tx).await.map_err(db)?;
        if !auth {
            return Ok(ChangeStoreOutcome::NotAuthorized);
        }
        let r=sqlx::query("DELETE FROM conversation_members WHERE conversation_id=? AND user_id=? AND role IN ('admin','member')").bind(raw(id)).bind(raw(member)).execute(&mut *tx).await.map_err(db)?;
        if r.rows_affected() > 0 {
            sqlx::query("UPDATE conversations SET updated_at=? WHERE id=?")
                .bind(micros(now).map_err(bad)?)
                .bind(raw(id))
                .execute(&mut *tx)
                .await
                .map_err(db)?;
        }
        tx.commit().await.map_err(db)?;
        Ok(if r.rows_affected() > 0 {
            ChangeStoreOutcome::Changed { updated_at: at }
        } else {
            ChangeStoreOutcome::Unchanged
        })
    }
    async fn set_group_member_role(
        &self,
        id: ConversationId,
        actor: UserId,
        member: UserId,
        role: GroupRole,
        now: OffsetDateTime,
    ) -> Result<ChangeStoreOutcome, ConversationStoreError> {
        let at = normalized(now)?;
        let role = match role {
            GroupRole::Admin => "admin",
            GroupRole::Member => "member",
            GroupRole::Owner => return Err(ConversationStoreError::InvalidGroupRole),
        };
        let mut tx = self.pool.begin().await.map_err(db)?;
        if !lock(&mut tx, id).await? {
            return Ok(ChangeStoreOutcome::NotAuthorized);
        }
        let auth:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM conversation_members WHERE conversation_id=? AND user_id=? AND role IN ('owner','admin'))").bind(raw(id)).bind(raw(actor)).fetch_one(&mut *tx).await.map_err(db)?;
        if !auth {
            return Ok(ChangeStoreOutcome::NotAuthorized);
        }
        let r=sqlx::query("UPDATE conversation_members SET role=? WHERE conversation_id=? AND user_id=? AND role IN ('admin','member') AND role<>?").bind(role).bind(raw(id)).bind(raw(member)).bind(role).execute(&mut *tx).await.map_err(db)?;
        if r.rows_affected() > 0 {
            sqlx::query("UPDATE conversations SET updated_at=? WHERE id=?")
                .bind(micros(now).map_err(bad)?)
                .bind(raw(id))
                .execute(&mut *tx)
                .await
                .map_err(db)?;
        }
        tx.commit().await.map_err(db)?;
        Ok(if r.rows_affected() > 0 {
            ChangeStoreOutcome::Changed { updated_at: at }
        } else {
            ChangeStoreOutcome::Unchanged
        })
    }
    async fn transfer_group_ownership(
        &self,
        id: ConversationId,
        actor: UserId,
        target: UserId,
        now: OffsetDateTime,
    ) -> Result<ChangeStoreOutcome, ConversationStoreError> {
        let at = normalized(now)?;
        let mut tx = self.pool.begin().await.map_err(db)?;
        if !lock(&mut tx, id).await? {
            return Ok(ChangeStoreOutcome::NotAuthorized);
        }
        let owner:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM conversation_members WHERE conversation_id=? AND user_id=? AND role='owner')").bind(raw(id)).bind(raw(actor)).fetch_one(&mut *tx).await.map_err(db)?;
        if !owner {
            return Ok(ChangeStoreOutcome::NotAuthorized);
        }
        if actor == target {
            tx.commit().await.map_err(db)?;
            return Ok(ChangeStoreOutcome::Unchanged);
        }
        let target_exists:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM conversation_members WHERE conversation_id=? AND user_id=? AND role IN ('admin','member'))").bind(raw(id)).bind(raw(target)).fetch_one(&mut *tx).await.map_err(db)?;
        if !target_exists {
            return Ok(ChangeStoreOutcome::Unchanged);
        }
        sqlx::query("UPDATE conversation_members SET role='admin' WHERE conversation_id=? AND user_id=? AND role='owner'").bind(raw(id)).bind(raw(actor)).execute(&mut *tx).await.map_err(db)?;
        sqlx::query(
            "UPDATE conversation_members SET role='owner' WHERE conversation_id=? AND user_id=?",
        )
        .bind(raw(id))
        .bind(raw(target))
        .execute(&mut *tx)
        .await
        .map_err(db)?;
        sqlx::query("UPDATE conversations SET updated_at=? WHERE id=?")
            .bind(micros(now).map_err(bad)?)
            .bind(raw(id))
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        tx.commit().await.map_err(db)?;
        Ok(ChangeStoreOutcome::Changed { updated_at: at })
    }
    async fn leave_group(
        &self,
        id: ConversationId,
        actor: UserId,
        now: OffsetDateTime,
    ) -> Result<LeaveStoreOutcome, ConversationStoreError> {
        let at = normalized(now)?;
        let mut tx = self.pool.begin().await.map_err(db)?;
        let row = sqlx::query("SELECT kind FROM conversations WHERE id=? FOR UPDATE")
            .bind(raw(id))
            .fetch_optional(&mut *tx)
            .await
            .map_err(db)?;
        let Some(row) = row else {
            return Ok(LeaveStoreOutcome::NotMember);
        };
        if row.try_get::<String, _>("kind").map_err(db)? == "direct" {
            return Ok(LeaveStoreOutcome::Direct);
        }
        let role: Option<String> = sqlx::query_scalar(
            "SELECT role FROM conversation_members WHERE conversation_id=? AND user_id=?",
        )
        .bind(raw(id))
        .bind(raw(actor))
        .fetch_optional(&mut *tx)
        .await
        .map_err(db)?;
        let Some(role) = role else {
            return Ok(LeaveStoreOutcome::NotMember);
        };
        if role == "owner" {
            let n: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM conversation_members WHERE conversation_id=?",
            )
            .bind(raw(id))
            .fetch_one(&mut *tx)
            .await
            .map_err(db)?;
            if n != 1 {
                return Ok(LeaveStoreOutcome::OwnershipTransferRequired);
            }
            // InnoDB does not cascade through the self-referential reply FK.
            // Clear reply edges and remove history before deleting the parent.
            sqlx::query("UPDATE messages SET reply_to_message_id=NULL WHERE conversation_id=?")
                .bind(raw(id))
                .execute(&mut *tx)
                .await
                .map_err(db)?;
            sqlx::query("DELETE FROM messages WHERE conversation_id=?")
                .bind(raw(id))
                .execute(&mut *tx)
                .await
                .map_err(db)?;
            sqlx::query("DELETE FROM conversation_members WHERE conversation_id=?")
                .bind(raw(id))
                .execute(&mut *tx)
                .await
                .map_err(db)?;
            sqlx::query("DELETE FROM conversations WHERE id=?")
                .bind(raw(id))
                .execute(&mut *tx)
                .await
                .map_err(db)?;
            tx.commit().await.map_err(db)?;
            return Ok(LeaveStoreOutcome::ConversationDeleted);
        }
        sqlx::query("DELETE FROM conversation_members WHERE conversation_id=? AND user_id=?")
            .bind(raw(id))
            .bind(raw(actor))
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        sqlx::query("UPDATE conversations SET updated_at=? WHERE id=?")
            .bind(micros(now).map_err(bad)?)
            .bind(raw(id))
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        tx.commit().await.map_err(db)?;
        Ok(LeaveStoreOutcome::Left { updated_at: at })
    }
    async fn rename_group(
        &self,
        id: ConversationId,
        actor: UserId,
        title: &GroupTitle,
        now: OffsetDateTime,
    ) -> Result<ChangeStoreOutcome, ConversationStoreError> {
        let at = normalized(now)?;
        let mut tx = self.pool.begin().await.map_err(db)?;
        if !lock(&mut tx, id).await? {
            return Ok(ChangeStoreOutcome::NotAuthorized);
        }
        let auth:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM conversation_members WHERE conversation_id=? AND user_id=? AND role IN ('owner','admin'))").bind(raw(id)).bind(raw(actor)).fetch_one(&mut *tx).await.map_err(db)?;
        if !auth {
            return Ok(ChangeStoreOutcome::NotAuthorized);
        }
        let old: Option<Vec<u8>> =
            sqlx::query_scalar("SELECT title FROM conversations WHERE id=? AND kind='group'")
                .bind(raw(id))
                .fetch_optional(&mut *tx)
                .await
                .map_err(db)?;
        let Some(old) = old else {
            return Ok(ChangeStoreOutcome::NotAuthorized);
        };
        if old == title.as_str().as_bytes() {
            tx.commit().await.map_err(db)?;
            return Ok(ChangeStoreOutcome::Unchanged);
        }
        sqlx::query("UPDATE conversations SET title=?,updated_at=? WHERE id=?")
            .bind(title.as_str().as_bytes())
            .bind(micros(now).map_err(bad)?)
            .bind(raw(id))
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        tx.commit().await.map_err(db)?;
        Ok(ChangeStoreOutcome::Changed { updated_at: at })
    }
}
