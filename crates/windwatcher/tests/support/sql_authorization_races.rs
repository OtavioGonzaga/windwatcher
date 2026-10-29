// Exercise the same deterministic lock interleaving against both SQL engines.
macro_rules! authorization_races {
    ($storage:expr, $pool:expr, $db:ty, $encode:expr, $owner:expr, $actor:expr) => {{
        use windwatcher::*;
        let storage = $storage;
        let pool = $pool;
        let encode = $encode;
        let owner = $owner;
        let actor = $actor;
        let now = time::OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
        // Author edit, author delete, moderator demotion and moderator removal.
        for case in 0..4 {
            let group = Conversation::new_group(
                ConversationId::new(),
                GroupTitle::new("lock race").unwrap(),
                now,
            );
            storage.create_group(&group, owner, &[actor]).await.unwrap();
            if case >= 2 {
                storage
                    .set_group_member_role(group.id(), owner, actor, GroupRole::Admin, now)
                    .await
                    .unwrap();
            }
            let sender = if case < 2 { actor } else { owner };
            let message = Message::new(
                MessageId::new(),
                group.id(),
                sender,
                MessageContent::new("original").unwrap(),
                None,
                now,
            )
            .unwrap();
            storage.send_message(&message).await.unwrap();
            let mut tx = pool.begin().await.unwrap();
            let mut lock = sqlx::QueryBuilder::<$db>::new("SELECT id FROM conversations WHERE id=");
            lock.push_bind(encode(group.id().as_uuid()))
                .push(" FOR UPDATE");
            lock.build().fetch_one(&mut *tx).await.unwrap();
            let replacement = MessageContent::new("forbidden edit").unwrap();
            let mutation = async {
                if case == 0 {
                    assert_eq!(
                        storage
                            .edit_message(actor, message.id(), &replacement, now)
                            .await
                            .unwrap(),
                        EditMessageStoreOutcome::NotMember
                    );
                } else {
                    let result = storage
                        .delete_message(actor, message.id(), now)
                        .await
                        .unwrap();
                    assert_eq!(
                        result,
                        if case == 2 {
                            DeleteMessageStoreOutcome::NotAuthorized
                        } else {
                            DeleteMessageStoreOutcome::NotMember
                        }
                    );
                }
            };
            tokio::pin!(mutation);
            // The mutation must wait at the conversation lock, before authorization.
            assert!(
                tokio::time::timeout(std::time::Duration::from_millis(100), &mut mutation)
                    .await
                    .is_err()
            );
            let mut revoke = sqlx::QueryBuilder::<$db>::new(if case == 2 {
                "UPDATE conversation_members SET role='member' WHERE conversation_id="
            } else {
                "DELETE FROM conversation_members WHERE conversation_id="
            });
            revoke
                .push_bind(encode(group.id().as_uuid()))
                .push(" AND user_id=")
                .push_bind(encode(actor.as_uuid()));
            revoke.build().execute(&mut *tx).await.unwrap();
            tx.commit().await.unwrap();
            tokio::time::timeout(std::time::Duration::from_secs(5), mutation)
                .await
                .unwrap();
            match storage.get_message(owner, message.id()).await.unwrap() {
                GetMessageStoreOutcome::Found(view) => assert_eq!(view.message, message),
                other => panic!("unexpected message after rejected mutation: {other:?}"),
            }
        }
    }};
}
