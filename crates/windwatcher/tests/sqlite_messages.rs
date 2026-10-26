#![cfg(feature = "sqlite")]

use sqlx::{Row, SqlitePool, sqlite::SqliteConnectOptions};
use tempfile::TempDir;
use time::OffsetDateTime;
use windwatcher::*;

struct Fixture {
    _directory: TempDir,
    store: SqliteStorage,
    pool: SqlitePool,
    owner: UserId,
    peer: UserId,
    conversation: ConversationId,
}

fn bytes(id: impl Into<uuid::Uuid>) -> Vec<u8> {
    id.into().into_bytes().to_vec()
}
fn now() -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap()
}

async fn fixture() -> Fixture {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("messages.db");
    let store = SqliteStorage::connect(
        SqliteStorageOptions::new(path.display().to_string()).max_connections(4),
    )
    .await
    .unwrap();
    store.migrate().await.unwrap();
    let pool = SqlitePool::connect_with(
        SqliteConnectOptions::new()
            .filename(path)
            .foreign_keys(true),
    )
    .await
    .unwrap();
    let hash = PasswordHash::parse(
        "$argon2id$v=19$m=32,t=1,p=1$dGVzdC1vbmx5LXNhbHQ$AAAAAAAAAAAAAAAAAAAAAA",
    )
    .unwrap();
    let owner = User::new(UserId::new(), "owner".parse().unwrap(), now());
    let peer = User::new(UserId::new(), "peer".parse().unwrap(), now());
    store.create_local_user(&owner, &hash).await.unwrap();
    store.create_local_user(&peer, &hash).await.unwrap();
    let conversation = Conversation::new_group(
        ConversationId::new(),
        GroupTitle::new("messages").unwrap(),
        now(),
    );
    store
        .create_group(&conversation, owner.id(), &[peer.id()])
        .await
        .unwrap();
    Fixture {
        _directory: directory,
        store,
        pool,
        owner: owner.id(),
        peer: peer.id(),
        conversation: conversation.id(),
    }
}

async fn send(f: &Fixture, reply: Option<MessageId>) -> Message {
    SendMessage::new(&f.store, &MessagePolicy::default())
        .execute(f.owner, f.conversation, "message", reply, now())
        .await
        .unwrap()
}

#[tokio::test]
async fn pagination_reads_without_acquiring_the_writer_lock() {
    let f = fixture().await;
    let message = send(&f, None).await;
    let mut writer = f.pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
    sqlx::query("DELETE FROM conversation_members WHERE conversation_id = ? AND user_id = ?")
        .bind(bytes(f.conversation))
        .bind(bytes(f.peer))
        .execute(&mut *writer)
        .await
        .unwrap();
    // An uncommitted writer must not prevent an authorized read of committed history.
    let ListMessagesStoreOutcome::Page(page) = f
        .store
        .list_messages(f.peer, f.conversation, None, PageLimit::DEFAULT)
        .await
        .unwrap()
    else {
        panic!("expected committed membership snapshot")
    };
    assert_eq!(page.items, [message]);
    writer.commit().await.unwrap();
    assert_eq!(
        f.store
            .list_messages(f.peer, f.conversation, None, PageLimit::DEFAULT)
            .await
            .unwrap(),
        ListMessagesStoreOutcome::NotMember
    );
}

#[tokio::test]
async fn schema_enforces_physical_types_checks_and_reply_integrity() {
    let f = fixture().await;
    let strict: i64 =
        sqlx::query_scalar("SELECT strict FROM pragma_table_list WHERE name = 'messages'")
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(strict, 1);
    let columns = sqlx::query("PRAGMA table_info(messages)")
        .fetch_all(&f.pool)
        .await
        .unwrap();
    for (column, expected) in [
        ("id", "BLOB"),
        ("conversation_id", "BLOB"),
        ("sender_id", "BLOB"),
        ("reply_to_message_id", "BLOB"),
        ("content", "TEXT"),
        ("created_at", "INTEGER"),
        ("edited_at", "INTEGER"),
        ("deleted_at", "INTEGER"),
    ] {
        let row = columns
            .iter()
            .find(|r| r.get::<String, _>("name") == column)
            .unwrap();
        assert_eq!(row.get::<String, _>("type"), expected);
        if column == "content" {
            assert_eq!(row.get::<i64, _>("notnull"), 0);
        }
    }
    let marker = sqlx::query("SELECT type, [notnull] FROM pragma_table_info('conversation_members') WHERE name = 'last_read_message_id'").fetch_one(&f.pool).await.unwrap();
    assert_eq!(marker.get::<String, _>("type"), "BLOB");
    assert_eq!(marker.get::<i64, _>("notnull"), 0);
    let fks = sqlx::query("PRAGMA foreign_key_list(messages)")
        .fetch_all(&f.pool)
        .await
        .unwrap();
    for (table, action) in [
        ("conversations", "CASCADE"),
        ("users", "RESTRICT"),
        ("messages", "NO ACTION"),
    ] {
        assert!(
            fks.iter().any(|r| r.get::<String, _>("table") == table
                && r.get::<String, _>("on_delete") == action)
        );
    }
    assert!(fks.iter().any(|r| r.get::<String, _>("table") == "messages"
        && r.get::<String, _>("from") == "conversation_id"));
    assert!(fks.iter().any(|r| r.get::<String, _>("table") == "messages"
        && r.get::<String, _>("from") == "reply_to_message_id"));
    let indexes = sqlx::query("SELECT name FROM pragma_index_list('messages') WHERE [unique] = 1")
        .fetch_all(&f.pool)
        .await
        .unwrap();
    let mut ordering_index = false;
    for index in indexes {
        let columns: Vec<String> =
            sqlx::query_scalar("SELECT name FROM pragma_index_info(?) ORDER BY seqno")
                .bind(index.get::<String, _>("name"))
                .fetch_all(&f.pool)
                .await
                .unwrap();
        ordering_index |= columns == ["conversation_id", "id"];
    }
    assert!(ordering_index);
    let first = send(&f, None).await;
    let row = sqlx::query("SELECT typeof(id) AS id_type, length(id) AS id_len, typeof(created_at) AS time_type, created_at FROM messages WHERE id = ?").bind(bytes(first.id())).fetch_one(&f.pool).await.unwrap();
    assert_eq!(row.get::<String, _>("id_type"), "blob");
    assert_eq!(row.get::<i64, _>("id_len"), 16);
    assert_eq!(row.get::<String, _>("time_type"), "integer");
    assert_eq!(row.get::<i64, _>("created_at"), 1_700_000_000_000_000);
    for sql in [
        "UPDATE messages SET content = NULL WHERE id = ?",
        "UPDATE messages SET deleted_at = 1 WHERE id = ?",
        "UPDATE messages SET reply_to_message_id = id WHERE id = ?",
        "UPDATE messages SET sender_id = zeroblob(15) WHERE id = ?",
        "UPDATE messages SET id = zeroblob(15) WHERE id = ?",
        "UPDATE messages SET conversation_id = zeroblob(15) WHERE id = ?",
        "UPDATE messages SET reply_to_message_id = zeroblob(15) WHERE id = ?",
        "UPDATE messages SET created_at = 'not-an-integer' WHERE id = ?",
    ] {
        assert!(
            sqlx::query(sql)
                .bind(bytes(first.id()))
                .execute(&f.pool)
                .await
                .is_err(),
            "{sql}"
        );
    }
    assert!(
        sqlx::query("UPDATE conversation_members SET last_read_message_id = zeroblob(15)")
            .execute(&f.pool)
            .await
            .is_err()
    );
    let other = Conversation::new_group(
        ConversationId::new(),
        GroupTitle::new("other").unwrap(),
        now(),
    );
    f.store.create_group(&other, f.owner, &[]).await.unwrap();
    assert!(sqlx::query("INSERT INTO messages (id, conversation_id, sender_id, content, reply_to_message_id, created_at) VALUES (?, ?, ?, '', ?, 0)")
        .bind(bytes(MessageId::new())).bind(bytes(other.id())).bind(bytes(f.owner)).bind(bytes(first.id())).execute(&f.pool).await.is_err());
    assert!(
        sqlx::query("DELETE FROM users WHERE id = ?")
            .bind(bytes(f.owner))
            .execute(&f.pool)
            .await
            .is_err()
    );
    let reply = send(&f, Some(first.id())).await;
    assert!(
        sqlx::query("DELETE FROM messages WHERE id = ?")
            .bind(bytes(first.id()))
            .execute(&f.pool)
            .await
            .is_err()
    );
    f.store
        .leave_group(f.conversation, f.peer, now())
        .await
        .unwrap();
    assert_eq!(
        f.store
            .leave_group(f.conversation, f.owner, now())
            .await
            .unwrap(),
        LeaveStoreOutcome::ConversationDeleted
    );
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE id IN (?, ?)")
        .bind(bytes(first.id()))
        .bind(bytes(reply.id()))
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM conversation_members WHERE conversation_id = ?")
            .bind(bytes(f.conversation))
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn blob_order_matches_message_id_ord_and_read_queries() {
    let f = fixture().await;
    let mut ids: Vec<MessageId> = [
        "018f6f35-0000-7000-8000-000000000000",
        "018f6f34-0000-7001-8000-000000000000",
        "018f6f34-0000-7000-bfff-ffffffffffff",
        "018f6f34-0000-7000-8000-000000000000",
    ]
    .map(|text| text.parse().unwrap())
    .to_vec();
    for id in &ids {
        let message = Message::new(
            *id,
            f.conversation,
            f.owner,
            MessageContent::new("").unwrap(),
            None,
            now(),
        )
        .unwrap();
        assert!(matches!(
            f.store.send_message(&message).await.unwrap(),
            SendMessageStoreOutcome::Sent(_)
        ));
    }
    ids.sort();
    let persisted: Vec<Vec<u8>> =
        sqlx::query_scalar("SELECT id FROM messages WHERE conversation_id = ? ORDER BY id")
            .bind(bytes(f.conversation))
            .fetch_all(&f.pool)
            .await
            .unwrap();
    assert_eq!(
        persisted,
        ids.iter().copied().map(bytes).collect::<Vec<_>>()
    );
    let ListMessagesStoreOutcome::Page(page) = f
        .store
        .list_messages(f.peer, f.conversation, None, PageLimit::MAX)
        .await
        .unwrap()
    else {
        panic!("expected page")
    };
    assert_eq!(
        page.items.iter().map(Message::id).collect::<Vec<_>>(),
        ids.iter().rev().copied().collect::<Vec<_>>()
    );
    let ListMessagesStoreOutcome::Page(page) = f
        .store
        .list_messages(
            f.peer,
            f.conversation,
            Some(ids[2]),
            PageLimit::new(1).unwrap(),
        )
        .await
        .unwrap()
    else {
        panic!("expected page")
    };
    assert_eq!(page.items[0].id(), ids[1]);
    assert!(page.has_more);
    let plan: Vec<String> = sqlx::query("EXPLAIN QUERY PLAN SELECT id FROM messages WHERE conversation_id = ? AND id < ? ORDER BY id DESC LIMIT ?")
        .bind(bytes(f.conversation)).bind(bytes(ids[2])).bind(2_i64).fetch_all(&f.pool).await.unwrap()
        .iter().map(|row| row.get("detail")).collect();
    assert!(
        plan.iter()
            .any(|detail| detail.contains("SEARCH") && detail.contains("INDEX"))
    );
    f.store
        .mark_read_through(f.peer, f.conversation, ids[1])
        .await
        .unwrap();
    assert_eq!(
        f.store.unread_count(f.peer, f.conversation).await.unwrap(),
        UnreadCountStoreOutcome::Count(2)
    );
    f.store
        .mark_read_through(f.peer, f.conversation, ids[0])
        .await
        .unwrap();
    assert_eq!(
        f.store
            .find_member(f.conversation, f.peer)
            .await
            .unwrap()
            .unwrap()
            .last_read_message_id(),
        Some(ids[1])
    );
}

#[tokio::test]
async fn reply_delete_edit_and_read_races_preserve_invariants() {
    let f = fixture().await;
    for _ in 0..12 {
        let target = send(&f, None).await;
        let reply = Message::new(
            MessageId::new(),
            f.conversation,
            f.owner,
            MessageContent::new("reply").unwrap(),
            Some(target.id()),
            now(),
        )
        .unwrap();
        let (reply_result, deletion) = tokio::join!(
            f.store.send_message(&reply),
            f.store
                .delete_message(f.owner, f.conversation, target.id(), now()),
        );
        assert!(matches!(
            deletion.unwrap(),
            DeleteMessageStoreOutcome::Deleted(_)
        ));
        match reply_result.unwrap() {
            SendMessageStoreOutcome::Sent(persisted) => {
                assert_eq!(persisted.reply_to_message_id(), Some(target.id()));
                assert_eq!(
                    f.store
                        .get_message(f.owner, f.conversation, persisted.id())
                        .await
                        .unwrap(),
                    GetMessageStoreOutcome::Found(persisted)
                );
            }
            SendMessageStoreOutcome::ReplyTargetDeleted => {
                assert_eq!(
                    f.store
                        .get_message(f.owner, f.conversation, reply.id())
                        .await
                        .unwrap(),
                    GetMessageStoreOutcome::MessageNotFound
                );
            }
            other => panic!("invalid reply/delete outcome: {other:?}"),
        }
        // A reply attempted strictly after committed deletion must always fail.
        let after = Message::new(
            MessageId::new(),
            f.conversation,
            f.owner,
            MessageContent::new("after").unwrap(),
            Some(target.id()),
            now(),
        )
        .unwrap();
        assert_eq!(
            f.store.send_message(&after).await.unwrap(),
            SendMessageStoreOutcome::ReplyTargetDeleted
        );
        let editable = send(&f, None).await;
        let content = MessageContent::new("edited").unwrap();
        let (edit, delete) = tokio::join!(
            f.store
                .edit_message(f.owner, f.conversation, editable.id(), &content, now()),
            f.store
                .delete_message(f.owner, f.conversation, editable.id(), now()),
        );
        assert!(matches!(
            edit.unwrap(),
            EditMessageStoreOutcome::Changed(_) | EditMessageStoreOutcome::Deleted
        ));
        assert!(matches!(
            delete.unwrap(),
            DeleteMessageStoreOutcome::Deleted(_)
        ));
        let row = sqlx::query("SELECT content, deleted_at FROM messages WHERE id = ?")
            .bind(bytes(editable.id()))
            .fetch_one(&f.pool)
            .await
            .unwrap();
        assert_eq!(row.get::<Option<String>, _>("content"), None);
        assert!(row.get::<Option<i64>, _>("deleted_at").is_some());
        let older = send(&f, None).await;
        let newer = send(&f, None).await;
        let (a, b) = tokio::join!(
            f.store
                .mark_read_through(f.peer, f.conversation, newer.id()),
            f.store
                .mark_read_through(f.peer, f.conversation, older.id()),
        );
        assert!(matches!(a.unwrap(), MarkReadStoreOutcome::Changed));
        assert!(matches!(
            b.unwrap(),
            MarkReadStoreOutcome::Changed | MarkReadStoreOutcome::Unchanged
        ));
        assert_eq!(
            f.store
                .find_member(f.conversation, f.peer)
                .await
                .unwrap()
                .unwrap()
                .last_read_message_id(),
            Some(newer.id())
        );
    }
}

#[tokio::test]
async fn writes_observe_membership_and_role_changes_before_commit() {
    let f = fixture().await;
    let message = send(&f, None).await;
    f.store
        .set_group_member_role(f.conversation, f.owner, f.peer, GroupRole::Admin, now())
        .await
        .unwrap();
    let mut tx = f.pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
    sqlx::query(
        "UPDATE conversation_members SET role = 'member' WHERE conversation_id = ? AND user_id = ?",
    )
    .bind(bytes(f.conversation))
    .bind(bytes(f.peer))
    .execute(&mut *tx)
    .await
    .unwrap();
    let store = f.store.clone();
    let (actor, c, id) = (f.peer, f.conversation, message.id());
    let deletion = tokio::spawn(async move { store.delete_message(actor, c, id, now()).await });
    tx.commit().await.unwrap();
    assert_eq!(
        deletion.await.unwrap().unwrap(),
        DeleteMessageStoreOutcome::NotAuthorized
    );
    let mut tx = f.pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
    sqlx::query("DELETE FROM conversation_members WHERE conversation_id = ? AND user_id = ?")
        .bind(bytes(f.conversation))
        .bind(bytes(f.peer))
        .execute(&mut *tx)
        .await
        .unwrap();
    let store = f.store.clone();
    let attempt = Message::new(
        MessageId::new(),
        c,
        actor,
        MessageContent::new("stale membership").unwrap(),
        None,
        now(),
    )
    .unwrap();
    let sending = tokio::spawn(async move { store.send_message(&attempt).await });
    tx.commit().await.unwrap();
    assert_eq!(
        sending.await.unwrap().unwrap(),
        SendMessageStoreOutcome::NotMember
    );
    let content = MessageContent::new("edit").unwrap();
    assert_eq!(
        f.store
            .edit_message(actor, c, id, &content, now())
            .await
            .unwrap(),
        EditMessageStoreOutcome::NotMember
    );
    assert_eq!(
        f.store.mark_read_through(actor, c, id).await.unwrap(),
        MarkReadStoreOutcome::NotMember
    );
}

#[tokio::test]
async fn joining_and_sending_share_an_atomic_history_boundary() {
    let f = fixture().await;
    for _ in 0..12 {
        f.store
            .leave_group(f.conversation, f.peer, now())
            .await
            .unwrap();
        let prior = send(&f, None).await;
        let message = Message::new(
            MessageId::new(),
            f.conversation,
            f.owner,
            MessageContent::new("concurrent").unwrap(),
            None,
            now(),
        )
        .unwrap();
        let (joined, sent) = tokio::join!(
            f.store
                .add_group_member(f.conversation, f.owner, f.peer, now()),
            f.store.send_message(&message),
        );
        assert_eq!(joined.unwrap(), AddMemberStoreOutcome::Added);
        assert!(matches!(sent.unwrap(), SendMessageStoreOutcome::Sent(_)));
        let marker = f
            .store
            .find_member(f.conversation, f.peer)
            .await
            .unwrap()
            .unwrap()
            .last_read_message_id()
            .unwrap();
        assert!(marker == prior.id() || marker == message.id());
        let expected = if marker == prior.id() { 1 } else { 0 };
        assert_eq!(
            f.store.unread_count(f.peer, f.conversation).await.unwrap(),
            UnreadCountStoreOutcome::Count(expected)
        );
    }
}

#[tokio::test]
async fn storage_rolls_back_failures_rejects_corruption_and_loads_large_history() {
    let f = fixture().await;
    let policy = MessagePolicy::default();
    let large = SendMessage::new(&f.store, &policy)
        .execute(f.owner, f.conversation, "a".repeat(100_000), None, now())
        .await
        .unwrap();
    let smaller = MessagePolicy::new(64_000);
    assert!(smaller.validate(large.content().unwrap()).is_err());
    assert_eq!(
        GetMessage::new(&f.store)
            .execute(f.owner, f.conversation, large.id())
            .await
            .unwrap(),
        large
    );
    let invalid = Message::restore(
        MessageId::new(),
        f.conversation,
        f.owner,
        None,
        None,
        now(),
        None,
        Some(now()),
    )
    .unwrap();
    assert!(matches!(
        f.store.send_message(&invalid).await,
        Err(MessageStoreError::Unexpected(_))
    ));
    sqlx::query("CREATE TRIGGER reject_message_edit BEFORE UPDATE ON messages BEGIN SELECT RAISE(ABORT, 'test failure'); END;").execute(&f.pool).await.unwrap();
    assert!(
        f.store
            .edit_message(
                f.owner,
                f.conversation,
                large.id(),
                &MessageContent::new("changed").unwrap(),
                now()
            )
            .await
            .is_err()
    );
    assert_eq!(
        GetMessage::new(&f.store)
            .execute(f.owner, f.conversation, large.id())
            .await
            .unwrap(),
        large
    );
    sqlx::query("DROP TRIGGER reject_message_edit")
        .execute(&f.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE messages SET content = ? WHERE id = ?")
        .bind("bad\0content")
        .bind(bytes(large.id()))
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(matches!(
        f.store
            .get_message(f.owner, f.conversation, large.id())
            .await,
        Err(MessageStoreError::Unexpected(_))
    ));
    sqlx::query(
        "UPDATE conversation_members SET last_read_message_id = zeroblob(16) WHERE user_id = ?",
    )
    .bind(bytes(f.peer))
    .execute(&f.pool)
    .await
    .unwrap();
    assert!(matches!(
        f.store.find_member(f.conversation, f.peer).await,
        Err(ConversationStoreError::Unexpected(_))
    ));
}

#[tokio::test]
async fn migration_upgrades_existing_memberships_without_backfill() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("upgrade.db");
    let pool = SqlitePool::connect_with(
        SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true)
            .foreign_keys(true),
    )
    .await
    .unwrap();
    let mut migrator = sqlx::migrate!("./migrations/sqlite");
    migrator.migrations =
        std::borrow::Cow::Owned(migrator.iter().filter(|m| m.version < 3).cloned().collect());
    migrator.run(&pool).await.unwrap();
    let owner = UserId::new();
    let c = ConversationId::new();
    sqlx::query(
        "INSERT INTO users (id, username, created_at, updated_at) VALUES (?, 'existing', 0, 0)",
    )
    .bind(bytes(owner))
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO conversations (id, kind, title, created_at, updated_at) VALUES (?, 'group', 'existing', 0, 0)").bind(bytes(c)).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO conversation_members (conversation_id, user_id, role, joined_at) VALUES (?, ?, 'owner', 0)").bind(bytes(c)).bind(bytes(owner)).execute(&pool).await.unwrap();
    let store = SqliteStorage::connect(SqliteStorageOptions::new(path.display().to_string()))
        .await
        .unwrap();
    assert_eq!(store.migration_status().await.unwrap().pending(), 1);
    store.migrate().await.unwrap();
    assert_eq!(
        store
            .find_member(c, owner)
            .await
            .unwrap()
            .unwrap()
            .last_read_message_id(),
        None
    );
    assert_eq!(
        store.unread_count(owner, c).await.unwrap(),
        UnreadCountStoreOutcome::Count(0)
    );
    assert!(store.migration_status().await.unwrap().is_current());
}
