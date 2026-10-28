use super::{PostgresStorage, date, micros, transient};
use crate::{
    AddMemberStoreOutcome, ChangeStoreOutcome, Conversation, ConversationId, ConversationKind,
    ConversationMember, ConversationStore, ConversationStoreError, GroupRole, GroupTitle,
    LeaveStoreOutcome, MembershipRole, StartDirectStoreOutcome, UserId,
};
use sqlx::{PgConnection, Row};
use std::{error::Error, fmt};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug)]
struct Invalid(&'static str);
impl fmt::Display for Invalid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl Error for Invalid {}
fn bad(s: &'static str) -> ConversationStoreError {
    ConversationStoreError::Unexpected(Box::new(Invalid(s)))
}
fn db(e: sqlx::Error) -> ConversationStoreError {
    if transient(&e) {
        ConversationStoreError::Unavailable(Box::new(e))
    } else if matches!(&e,sqlx::Error::Database(d)if d.is_unique_violation()||d.is_foreign_key_violation())
    {
        ConversationStoreError::Conflict
    } else {
        ConversationStoreError::Unexpected(Box::new(e))
    }
}
fn normalize(t: OffsetDateTime) -> Result<OffsetDateTime, ConversationStoreError> {
    let n = micros(t).map_err(bad)?;
    date(n).map_err(bad)
}
fn read_conversation(r: &sqlx::postgres::PgRow) -> Result<Conversation, ConversationStoreError> {
    let id = ConversationId::try_from(r.try_get::<Uuid, _>("id").map_err(db)?)
        .map_err(|_| bad("invalid persisted conversation ID"))?;
    let kind = match r.try_get::<String, _>("kind").map_err(db)?.as_str() {
        "direct" => ConversationKind::Direct,
        "group" => ConversationKind::Group,
        _ => return Err(bad("invalid persisted conversation kind")),
    };
    let title: Option<Vec<u8>> = r.try_get("title").map_err(db)?;
    let title = title
        .map(|b| {
            String::from_utf8(b)
                .map_err(|_| bad("invalid UTF-8 conversation title"))
                .and_then(|s| {
                    GroupTitle::new(s).map_err(|_| bad("invalid persisted conversation title"))
                })
        })
        .transpose()?;
    let created = date(r.try_get("created_at").map_err(db)?).map_err(bad)?;
    let updated = date(r.try_get("updated_at").map_err(db)?).map_err(bad)?;
    Conversation::restore(id, kind, title, created, updated)
        .map_err(|_| bad("incoherent persisted conversation"))
}
fn read_member(r: &sqlx::postgres::PgRow) -> Result<ConversationMember, ConversationStoreError> {
    let cid = ConversationId::try_from(r.try_get::<Uuid, _>("conversation_id").map_err(db)?)
        .map_err(|_| bad("invalid conversation ID"))?;
    let uid = UserId::try_from(r.try_get::<Uuid, _>("user_id").map_err(db)?)
        .map_err(|_| bad("invalid user ID"))?;
    let role = match r.try_get::<String, _>("role").map_err(db)?.as_str() {
        "direct" => MembershipRole::Direct,
        "owner" => MembershipRole::Group(GroupRole::Owner),
        "admin" => MembershipRole::Group(GroupRole::Admin),
        "member" => MembershipRole::Group(GroupRole::Member),
        _ => return Err(bad("invalid persisted membership role")),
    };
    let joined = date(r.try_get("joined_at").map_err(db)?).map_err(bad)?;
    let marker: Option<Uuid> = r.try_get("last_read_message_id").map_err(db)?;
    let marker = marker
        .map(|v| crate::MessageId::try_from(v).map_err(|_| bad("invalid persisted read marker")))
        .transpose()?;
    Ok(ConversationMember::restore(cid, uid, role, joined, marker))
}
fn key(a: UserId, b: UserId) -> Vec<u8> {
    let (mut a, mut b) = (a.as_uuid().into_bytes(), b.as_uuid().into_bytes());
    if a > b {
        std::mem::swap(&mut a, &mut b)
    }
    [a.as_slice(), b.as_slice()].concat()
}
async fn lock_conversation(
    c: &mut PgConnection,
    id: ConversationId,
) -> Result<bool, ConversationStoreError> {
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM conversations WHERE id=$1 FOR UPDATE)")
            .bind(id.as_uuid())
            .fetch_one(c)
            .await
            .map_err(db)?;
    Ok(exists)
}
async fn enabled(c: &mut PgConnection, id: UserId) -> Result<(), ConversationStoreError> {
    let state: Option<Option<i64>> =
        sqlx::query_scalar("SELECT disabled_at FROM users WHERE id=$1 FOR UPDATE")
            .bind(id.as_uuid())
            .fetch_optional(c)
            .await
            .map_err(db)?;
    if matches!(state, Some(None)) {
        Ok(())
    } else {
        Err(ConversationStoreError::ParticipantUnavailable(id))
    }
}
async fn create_direct(
    c: &mut PgConnection,
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
    let direct_key = key(a, b);
    if let Some(r) = sqlx::query(
        "SELECT id,kind,title,created_at,updated_at FROM conversations WHERE direct_key=$1",
    )
    .bind(&direct_key)
    .fetch_optional(&mut *c)
    .await
    .map_err(db)?
    {
        return Ok(StartDirectStoreOutcome::Existing(read_conversation(&r)?));
    }
    let mut users = vec![a];
    if b != a {
        users.push(b)
    }
    users.sort();
    for id in &users {
        enabled(c, *id).await?
    }
    let created = micros(conv.created_at()).map_err(bad)?;
    let updated = micros(conv.updated_at()).map_err(bad)?;
    let inserted=sqlx::query("INSERT INTO conversations(id,kind,title,direct_key,created_at,updated_at) VALUES($1,'direct',NULL,$2,$3,$4) ON CONFLICT(direct_key) DO NOTHING").bind(conv.id().as_uuid()).bind(&direct_key).bind(created).bind(updated).execute(&mut *c).await.map_err(db)?;
    if inserted.rows_affected() == 1 {
        for uid in [a, b] {
            sqlx::query("INSERT INTO conversation_members(conversation_id,user_id,role,joined_at) VALUES($1,$2,'direct',$3) ON CONFLICT(conversation_id,user_id) DO NOTHING").bind(conv.id().as_uuid()).bind(uid.as_uuid()).bind(created).execute(&mut *c).await.map_err(db)?;
        }
        Ok(StartDirectStoreOutcome::Created(conv.clone()))
    } else {
        let r = sqlx::query(
            "SELECT id,kind,title,created_at,updated_at FROM conversations WHERE direct_key=$1",
        )
        .bind(&direct_key)
        .fetch_one(&mut *c)
        .await
        .map_err(db)?;
        Ok(StartDirectStoreOutcome::Existing(read_conversation(&r)?))
    }
}
async fn create_group(
    c: &mut PgConnection,
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
    for id in &users {
        enabled(c, *id).await?
    }
    let t = micros(conv.created_at()).map_err(bad)?;
    sqlx::query("INSERT INTO conversations(id,kind,title,direct_key,created_at,updated_at) VALUES($1,'group',$2,NULL,$3,$4)").bind(conv.id().as_uuid()).bind(title.as_str().as_bytes()).bind(t).bind(micros(conv.updated_at()).map_err(bad)?).execute(&mut *c).await.map_err(db)?;
    sqlx::query("INSERT INTO conversation_members(conversation_id,user_id,role,joined_at) VALUES($1,$2,'owner',$3)").bind(conv.id().as_uuid()).bind(owner.as_uuid()).bind(t).execute(&mut *c).await.map_err(db)?;
    for uid in users {
        if uid != owner {
            sqlx::query("INSERT INTO conversation_members(conversation_id,user_id,role,joined_at) VALUES($1,$2,'member',$3) ON CONFLICT(conversation_id,user_id) DO NOTHING").bind(conv.id().as_uuid()).bind(uid.as_uuid()).bind(t).execute(&mut *c).await.map_err(db)?;
        }
    }
    Ok(())
}
impl ConversationStore for PostgresStorage {
    async fn member_ids(&self, id: ConversationId) -> Result<Vec<UserId>, ConversationStoreError> {
        let rows = sqlx::query("SELECT user_id FROM conversation_members WHERE conversation_id=$1")
            .bind(id.as_uuid())
            .fetch_all(&self.pool)
            .await
            .map_err(db)?;
        rows.into_iter()
            .map(|r| {
                UserId::try_from(r.try_get::<Uuid, _>("user_id").map_err(db)?)
                    .map_err(|_| bad("invalid user ID"))
            })
            .collect()
    }
    async fn get_or_create_direct(
        &self,
        c: &Conversation,
        a: UserId,
        b: UserId,
    ) -> Result<StartDirectStoreOutcome, ConversationStoreError> {
        let mut tx = self.pool.begin().await.map_err(db)?;
        let result = create_direct(&mut tx, c, a, b).await?;
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
        sqlx::query("SELECT id,kind,title,created_at,updated_at FROM conversations WHERE id=$1")
            .bind(id.as_uuid())
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
        sqlx::query("SELECT m.conversation_id,m.user_id,m.role,m.joined_at,m.last_read_message_id,c.kind FROM conversation_members m JOIN conversations c ON c.id=m.conversation_id WHERE m.conversation_id=$1 AND m.user_id=$2").bind(c.as_uuid()).bind(u.as_uuid()).fetch_optional(&self.pool).await.map_err(db)?.map(|r|{let m=read_member(&r)?;let kind:String=r.try_get("kind").map_err(db)?;if matches!((kind.as_str(),m.role()),("direct",MembershipRole::Direct)|("group",MembershipRole::Group(_))){Ok(m)}else{Err(bad("incompatible persisted membership role"))}}).transpose()
    }
    async fn add_group_member(
        &self,
        id: ConversationId,
        actor: UserId,
        member: UserId,
        now: OffsetDateTime,
    ) -> Result<AddMemberStoreOutcome, ConversationStoreError> {
        let normalized = normalize(now)?;
        let mut tx = self.pool.begin().await.map_err(db)?;
        if !lock_conversation(&mut tx, id).await? {
            return Ok(AddMemberStoreOutcome::NotAuthorized);
        }
        let auth:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM conversation_members WHERE conversation_id=$1 AND user_id=$2 AND role IN ('owner','admin'))").bind(id.as_uuid()).bind(actor.as_uuid()).fetch_one(&mut *tx).await.map_err(db)?;
        if !auth {
            return Ok(AddMemberStoreOutcome::NotAuthorized);
        }
        let exists:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM conversation_members WHERE conversation_id=$1 AND user_id=$2)").bind(id.as_uuid()).bind(member.as_uuid()).fetch_one(&mut *tx).await.map_err(db)?;
        if exists {
            tx.commit().await.map_err(db)?;
            return Ok(AddMemberStoreOutcome::AlreadyMember);
        }
        enabled(&mut tx, member).await?;
        let latest: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM messages WHERE conversation_id=$1 ORDER BY id DESC LIMIT 1",
        )
        .bind(id.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(db)?;
        sqlx::query("INSERT INTO conversation_members(conversation_id,user_id,role,joined_at,last_read_message_id) VALUES($1,$2,'member',$3,$4)").bind(id.as_uuid()).bind(member.as_uuid()).bind(micros(now).map_err(bad)?).bind(latest).execute(&mut *tx).await.map_err(db)?;
        sqlx::query("UPDATE conversations SET updated_at=$1 WHERE id=$2")
            .bind(micros(now).map_err(bad)?)
            .bind(id.as_uuid())
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        tx.commit().await.map_err(db)?;
        Ok(AddMemberStoreOutcome::Added {
            updated_at: normalized,
        })
    }
    async fn remove_group_member(
        &self,
        id: ConversationId,
        actor: UserId,
        member: UserId,
        now: OffsetDateTime,
    ) -> Result<ChangeStoreOutcome, ConversationStoreError> {
        let normalized = normalize(now)?;
        let mut tx = self.pool.begin().await.map_err(db)?;
        if !lock_conversation(&mut tx, id).await? {
            return Ok(ChangeStoreOutcome::NotAuthorized);
        }
        let auth:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM conversation_members WHERE conversation_id=$1 AND user_id=$2 AND role IN ('owner','admin'))").bind(id.as_uuid()).bind(actor.as_uuid()).fetch_one(&mut *tx).await.map_err(db)?;
        if !auth {
            return Ok(ChangeStoreOutcome::NotAuthorized);
        }
        let r=sqlx::query("DELETE FROM conversation_members WHERE conversation_id=$1 AND user_id=$2 AND role IN ('admin','member')").bind(id.as_uuid()).bind(member.as_uuid()).execute(&mut *tx).await.map_err(db)?;
        if r.rows_affected() == 1 {
            sqlx::query("UPDATE conversations SET updated_at=$1 WHERE id=$2")
                .bind(micros(now).map_err(bad)?)
                .bind(id.as_uuid())
                .execute(&mut *tx)
                .await
                .map_err(db)?;
        }
        tx.commit().await.map_err(db)?;
        Ok(if r.rows_affected() == 1 {
            ChangeStoreOutcome::Changed {
                updated_at: normalized,
            }
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
        let normalized = normalize(now)?;
        let value = match role {
            GroupRole::Admin => "admin",
            GroupRole::Member => "member",
            GroupRole::Owner => return Err(ConversationStoreError::InvalidGroupRole),
        };
        let mut tx = self.pool.begin().await.map_err(db)?;
        if !lock_conversation(&mut tx, id).await? {
            return Ok(ChangeStoreOutcome::NotAuthorized);
        }
        let auth:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM conversation_members WHERE conversation_id=$1 AND user_id=$2 AND role IN ('owner','admin'))").bind(id.as_uuid()).bind(actor.as_uuid()).fetch_one(&mut *tx).await.map_err(db)?;
        if !auth {
            return Ok(ChangeStoreOutcome::NotAuthorized);
        }
        let r=sqlx::query("UPDATE conversation_members SET role=$1 WHERE conversation_id=$2 AND user_id=$3 AND role IN ('admin','member') AND role<>$1").bind(value).bind(id.as_uuid()).bind(member.as_uuid()).execute(&mut *tx).await.map_err(db)?;
        if r.rows_affected() == 1 {
            sqlx::query("UPDATE conversations SET updated_at=$1 WHERE id=$2")
                .bind(micros(now).map_err(bad)?)
                .bind(id.as_uuid())
                .execute(&mut *tx)
                .await
                .map_err(db)?;
        }
        tx.commit().await.map_err(db)?;
        Ok(if r.rows_affected() == 1 {
            ChangeStoreOutcome::Changed {
                updated_at: normalized,
            }
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
        let normalized = normalize(now)?;
        let mut tx = self.pool.begin().await.map_err(db)?;
        if !lock_conversation(&mut tx, id).await? {
            return Ok(ChangeStoreOutcome::NotAuthorized);
        }
        let owner:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM conversation_members WHERE conversation_id=$1 AND user_id=$2 AND role='owner')").bind(id.as_uuid()).bind(actor.as_uuid()).fetch_one(&mut *tx).await.map_err(db)?;
        if !owner {
            return Ok(ChangeStoreOutcome::NotAuthorized);
        }
        if actor == target {
            tx.commit().await.map_err(db)?;
            return Ok(ChangeStoreOutcome::Unchanged);
        }
        let target_exists:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM conversation_members WHERE conversation_id=$1 AND user_id=$2 AND role IN ('admin','member'))").bind(id.as_uuid()).bind(target.as_uuid()).fetch_one(&mut *tx).await.map_err(db)?;
        if !target_exists {
            return Ok(ChangeStoreOutcome::Unchanged);
        }
        sqlx::query("UPDATE conversation_members SET role='admin' WHERE conversation_id=$1 AND user_id=$2 AND role='owner'").bind(id.as_uuid()).bind(actor.as_uuid()).execute(&mut *tx).await.map_err(db)?;
        sqlx::query(
            "UPDATE conversation_members SET role='owner' WHERE conversation_id=$1 AND user_id=$2",
        )
        .bind(id.as_uuid())
        .bind(target.as_uuid())
        .execute(&mut *tx)
        .await
        .map_err(db)?;
        sqlx::query("UPDATE conversations SET updated_at=$1 WHERE id=$2")
            .bind(micros(now).map_err(bad)?)
            .bind(id.as_uuid())
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        tx.commit().await.map_err(db)?;
        Ok(ChangeStoreOutcome::Changed {
            updated_at: normalized,
        })
    }
    async fn leave_group(
        &self,
        id: ConversationId,
        actor: UserId,
        now: OffsetDateTime,
    ) -> Result<LeaveStoreOutcome, ConversationStoreError> {
        let normalized = normalize(now)?;
        let mut tx = self.pool.begin().await.map_err(db)?;
        let row = sqlx::query("SELECT kind FROM conversations WHERE id=$1 FOR UPDATE")
            .bind(id.as_uuid())
            .fetch_optional(&mut *tx)
            .await
            .map_err(db)?;
        let Some(row) = row else {
            return Ok(LeaveStoreOutcome::NotMember);
        };
        let kind: String = row.try_get("kind").map_err(db)?;
        if kind == "direct" {
            return Ok(LeaveStoreOutcome::Direct);
        }
        let role: Option<String> = sqlx::query_scalar(
            "SELECT role FROM conversation_members WHERE conversation_id=$1 AND user_id=$2",
        )
        .bind(id.as_uuid())
        .bind(actor.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(db)?;
        let Some(role) = role else {
            return Ok(LeaveStoreOutcome::NotMember);
        };
        if role == "owner" {
            let count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM conversation_members WHERE conversation_id=$1",
            )
            .bind(id.as_uuid())
            .fetch_one(&mut *tx)
            .await
            .map_err(db)?;
            if count != 1 {
                return Ok(LeaveStoreOutcome::OwnershipTransferRequired);
            }
            sqlx::query("DELETE FROM conversations WHERE id=$1")
                .bind(id.as_uuid())
                .execute(&mut *tx)
                .await
                .map_err(db)?;
            tx.commit().await.map_err(db)?;
            return Ok(LeaveStoreOutcome::ConversationDeleted);
        }
        sqlx::query("DELETE FROM conversation_members WHERE conversation_id=$1 AND user_id=$2")
            .bind(id.as_uuid())
            .bind(actor.as_uuid())
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        sqlx::query("UPDATE conversations SET updated_at=$1 WHERE id=$2")
            .bind(micros(now).map_err(bad)?)
            .bind(id.as_uuid())
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        tx.commit().await.map_err(db)?;
        Ok(LeaveStoreOutcome::Left {
            updated_at: normalized,
        })
    }
    async fn rename_group(
        &self,
        id: ConversationId,
        actor: UserId,
        title: &GroupTitle,
        now: OffsetDateTime,
    ) -> Result<ChangeStoreOutcome, ConversationStoreError> {
        let normalized = normalize(now)?;
        let mut tx = self.pool.begin().await.map_err(db)?;
        if !lock_conversation(&mut tx, id).await? {
            return Ok(ChangeStoreOutcome::NotAuthorized);
        }
        let auth:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM conversation_members WHERE conversation_id=$1 AND user_id=$2 AND role IN ('owner','admin'))").bind(id.as_uuid()).bind(actor.as_uuid()).fetch_one(&mut *tx).await.map_err(db)?;
        if !auth {
            return Ok(ChangeStoreOutcome::NotAuthorized);
        }
        let existing: Option<Vec<u8>> =
            sqlx::query_scalar("SELECT title FROM conversations WHERE id=$1 AND kind='group'")
                .bind(id.as_uuid())
                .fetch_optional(&mut *tx)
                .await
                .map_err(db)?;
        let Some(old) = existing else {
            return Ok(ChangeStoreOutcome::NotAuthorized);
        };
        if old == title.as_str().as_bytes() {
            tx.commit().await.map_err(db)?;
            return Ok(ChangeStoreOutcome::Unchanged);
        }
        sqlx::query("UPDATE conversations SET title=$1,updated_at=$2 WHERE id=$3")
            .bind(title.as_str().as_bytes())
            .bind(micros(now).map_err(bad)?)
            .bind(id.as_uuid())
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        tx.commit().await.map_err(db)?;
        Ok(ChangeStoreOutcome::Changed {
            updated_at: normalized,
        })
    }
}
