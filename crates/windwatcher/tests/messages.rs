use futures_executor::block_on;
use time::OffsetDateTime;
use windwatcher::*;

#[derive(Clone)]
enum Response {
    Send(SendMessageStoreOutcome),
    Get(GetMessageStoreOutcome),
    Edit(EditMessageStoreOutcome),
    Delete(DeleteMessageStoreOutcome),
    Read(MarkReadStoreOutcome),
    Unread(UnreadCountStoreOutcome),
    Fail,
    MustNotCall,
    Echo(UserId, ConversationId),
}
struct Fake(Response);
impl MessageStore for Fake {
    async fn send_message(
        &self,
        message: &Message,
    ) -> Result<SendMessageStoreOutcome, MessageStoreError> {
        match &self.0 {
            Response::Send(value) => Ok(value.clone()),
            Response::Fail => Err(MessageStoreError::Conflict),
            Response::Echo(actor, conversation) => {
                assert_eq!(message.sender_id(), *actor);
                assert_eq!(message.conversation_id(), *conversation);
                assert_eq!(message.content().unwrap().as_str(), " 😀 ");
                assert_eq!(message.created_at(), OffsetDateTime::UNIX_EPOCH);
                Ok(SendMessageStoreOutcome::Sent(message.clone()))
            }
            _ => panic!("unexpected store call"),
        }
    }
    async fn get_message(
        &self,
        _: UserId,
        _: ConversationId,
        _: MessageId,
    ) -> Result<GetMessageStoreOutcome, MessageStoreError> {
        match &self.0 {
            Response::Get(value) => Ok(value.clone()),
            Response::Fail => Err(MessageStoreError::Conflict),
            _ => panic!("unexpected store call"),
        }
    }
    async fn edit_message(
        &self,
        _: UserId,
        _: ConversationId,
        _: MessageId,
        _: &MessageContent,
        _: OffsetDateTime,
    ) -> Result<EditMessageStoreOutcome, MessageStoreError> {
        match &self.0 {
            Response::Edit(value) => Ok(value.clone()),
            Response::Fail => Err(MessageStoreError::Conflict),
            _ => panic!("unexpected store call"),
        }
    }
    async fn delete_message(
        &self,
        _: UserId,
        _: ConversationId,
        _: MessageId,
        _: OffsetDateTime,
    ) -> Result<DeleteMessageStoreOutcome, MessageStoreError> {
        match &self.0 {
            Response::Delete(value) => Ok(value.clone()),
            Response::Fail => Err(MessageStoreError::Conflict),
            _ => panic!("unexpected store call"),
        }
    }
    async fn mark_read_through(
        &self,
        _: UserId,
        _: ConversationId,
        _: MessageId,
    ) -> Result<MarkReadStoreOutcome, MessageStoreError> {
        match &self.0 {
            Response::Read(value) => Ok(*value),
            Response::Fail => Err(MessageStoreError::Conflict),
            _ => panic!("unexpected store call"),
        }
    }
    async fn unread_count(
        &self,
        _: UserId,
        _: ConversationId,
    ) -> Result<UnreadCountStoreOutcome, MessageStoreError> {
        match &self.0 {
            Response::Unread(value) => Ok(*value),
            Response::Fail => Err(MessageStoreError::Conflict),
            _ => panic!("unexpected store call"),
        }
    }
}

#[test]
fn validates_writes_before_storage_and_preserves_actor_context() {
    block_on(async {
        let actor = UserId::new();
        let c = ConversationId::new();
        let id = MessageId::new();
        let now = OffsetDateTime::UNIX_EPOCH;
        let store = Fake(Response::MustNotCall);
        let policy = MessagePolicy::new(3);
        let send = SendMessage::new(&store, &policy);
        assert!(matches!(
            send.execute(actor, c, "\0", None, now).await,
            Err(SendMessageError::InvalidContent(
                MessageContentError::ContainsNul
            ))
        ));
        assert!(matches!(
            send.execute(actor, c, "😀", None, now).await,
            Err(SendMessageError::ContentTooLarge(
                MessagePolicyError::ContentTooLarge {
                    actual_bytes: 4,
                    max_bytes: 3
                }
            ))
        ));
        let edit = EditMessage::new(&store, &policy);
        assert!(matches!(
            edit.execute(actor, c, id, "\0", now).await,
            Err(EditMessageError::InvalidContent(_))
        ));
        assert!(matches!(
            edit.execute(actor, c, id, "😀", now).await,
            Err(EditMessageError::ContentTooLarge(_))
        ));
        let echo = Fake(Response::Echo(actor, c));
        let policy = MessagePolicy::default();
        let sent: Message = SendMessage::new(&echo, &policy)
            .execute(actor, c, " 😀 ", None, now)
            .await
            .unwrap();
        assert_eq!(sent.sender_id(), actor);
        assert!(!sent.is_deleted());
    });
}

#[test]
fn focused_errors_map_every_semantic_store_failure() {
    block_on(async {
        let actor = UserId::new();
        let c = ConversationId::new();
        let id = MessageId::new();
        let now = OffsetDateTime::UNIX_EPOCH;
        let policy = MessagePolicy::default();
        macro_rules! send_error {
            ($store:ident, $error:ident) => {
                assert!(matches!(
                    SendMessage::new(
                        &Fake(Response::Send(SendMessageStoreOutcome::$store)),
                        &policy
                    )
                    .execute(actor, c, "", None, now)
                    .await,
                    Err(SendMessageError::$error)
                ));
            };
        }
        send_error!(ConversationNotFound, ConversationNotFound);
        send_error!(NotMember, NotMember);
        send_error!(ReplyTargetNotFound, ReplyTargetNotFound);
        send_error!(ReplyTargetDeleted, ReplyTargetDeleted);
        macro_rules! get_error {
            ($variant:ident) => {
                assert!(matches!(
                    GetMessage::new(&Fake(Response::Get(GetMessageStoreOutcome::$variant)))
                        .execute(actor, c, id)
                        .await,
                    Err(GetMessageError::$variant)
                ));
            };
        }
        get_error!(ConversationNotFound);
        get_error!(NotMember);
        get_error!(MessageNotFound);
        macro_rules! edit_error {
            ($store:ident, $error:ident) => {
                assert!(matches!(
                    EditMessage::new(
                        &Fake(Response::Edit(EditMessageStoreOutcome::$store)),
                        &policy
                    )
                    .execute(actor, c, id, "", now)
                    .await,
                    Err(EditMessageError::$error)
                ));
            };
        }
        edit_error!(ConversationNotFound, ConversationNotFound);
        edit_error!(NotMember, NotMember);
        edit_error!(MessageNotFound, MessageNotFound);
        edit_error!(NotAuthor, NotAuthor);
        edit_error!(Deleted, MessageDeleted);
        macro_rules! delete_error {
            ($variant:ident) => {
                assert!(matches!(
                    DeleteMessage::new(&Fake(Response::Delete(
                        DeleteMessageStoreOutcome::$variant
                    )))
                    .execute(actor, c, id, now)
                    .await,
                    Err(DeleteMessageError::$variant)
                ));
            };
        }
        delete_error!(ConversationNotFound);
        delete_error!(NotMember);
        delete_error!(MessageNotFound);
        delete_error!(NotAuthorized);
        macro_rules! read_error {
            ($variant:ident) => {
                assert!(matches!(
                    MarkConversationReadThrough::new(&Fake(Response::Read(
                        MarkReadStoreOutcome::$variant
                    )))
                    .execute(actor, c, id)
                    .await,
                    Err(MarkConversationReadError::$variant)
                ));
            };
        }
        read_error!(ConversationNotFound);
        read_error!(NotMember);
        read_error!(MessageNotFound);
        assert!(matches!(
            GetUnreadCount::new(&Fake(Response::Unread(
                UnreadCountStoreOutcome::ConversationNotFound
            )))
            .execute(actor, c)
            .await,
            Err(GetUnreadCountError::ConversationNotFound)
        ));
        assert!(matches!(
            GetUnreadCount::new(&Fake(Response::Unread(UnreadCountStoreOutcome::NotMember)))
                .execute(actor, c)
                .await,
            Err(GetUnreadCountError::NotMember)
        ));
        let failed = Fake(Response::Fail);
        assert!(matches!(
            SendMessage::new(&failed, &policy)
                .execute(actor, c, "", None, now)
                .await,
            Err(SendMessageError::Storage(MessageStoreError::Conflict))
        ));
        assert!(matches!(
            GetMessage::new(&failed).execute(actor, c, id).await,
            Err(GetMessageError::Storage(_))
        ));
        assert!(matches!(
            EditMessage::new(&failed, &policy)
                .execute(actor, c, id, "", now)
                .await,
            Err(EditMessageError::Storage(_))
        ));
        assert!(matches!(
            DeleteMessage::new(&failed).execute(actor, c, id, now).await,
            Err(DeleteMessageError::Storage(_))
        ));
        assert!(matches!(
            MarkConversationReadThrough::new(&failed)
                .execute(actor, c, id)
                .await,
            Err(MarkConversationReadError::Storage(_))
        ));
        assert!(matches!(
            GetUnreadCount::new(&failed).execute(actor, c).await,
            Err(GetUnreadCountError::Storage(_))
        ));
    });
}

#[test]
fn public_outcomes_are_application_types_and_historical_reads_ignore_policy() {
    block_on(async {
        let actor = UserId::new();
        let c = ConversationId::new();
        let now = OffsetDateTime::UNIX_EPOCH;
        let message = Message::new(
            MessageId::new(),
            c,
            actor,
            MessageContent::new("historical".repeat(20_000)).unwrap(),
            None,
            now,
        )
        .unwrap();
        let policy = MessagePolicy::new(0);
        assert!(policy.validate(message.content().unwrap()).is_err());
        let loaded: Message = GetMessage::new(&Fake(Response::Get(GetMessageStoreOutcome::Found(
            message.clone(),
        ))))
        .execute(actor, c, message.id())
        .await
        .unwrap();
        assert_eq!(loaded, message);
        let edited = Message::restore(
            message.id(),
            c,
            actor,
            Some(MessageContent::new("").unwrap()),
            None,
            now,
            Some(now),
            None,
        )
        .unwrap();
        let tombstone = Message::restore(
            message.id(),
            c,
            actor,
            None,
            None,
            now,
            Some(now),
            Some(now),
        )
        .unwrap();
        for changed in [true, false] {
            let outcome = if changed {
                EditMessageStoreOutcome::Changed(edited.clone())
            } else {
                EditMessageStoreOutcome::Unchanged(edited.clone())
            };
            let result: EditMessageOutcome =
                EditMessage::new(&Fake(Response::Edit(outcome)), &policy)
                    .execute(actor, c, message.id(), "", now)
                    .await
                    .unwrap();
            assert_eq!(
                result,
                if changed {
                    EditMessageOutcome::Changed(edited.clone())
                } else {
                    EditMessageOutcome::Unchanged(edited.clone())
                }
            );
            let outcome = if changed {
                DeleteMessageStoreOutcome::Deleted(tombstone.clone())
            } else {
                DeleteMessageStoreOutcome::AlreadyDeleted(tombstone.clone())
            };
            let result: DeleteMessageOutcome = DeleteMessage::new(&Fake(Response::Delete(outcome)))
                .execute(actor, c, message.id(), now)
                .await
                .unwrap();
            assert_eq!(
                result,
                if changed {
                    DeleteMessageOutcome::Deleted(tombstone.clone())
                } else {
                    DeleteMessageOutcome::AlreadyDeleted(tombstone.clone())
                }
            );
            let result: MarkReadOutcome =
                MarkConversationReadThrough::new(&Fake(Response::Read(if changed {
                    MarkReadStoreOutcome::Changed
                } else {
                    MarkReadStoreOutcome::Unchanged
                })))
                .execute(actor, c, message.id())
                .await
                .unwrap();
            assert_eq!(
                result,
                if changed {
                    MarkReadOutcome::Changed
                } else {
                    MarkReadOutcome::Unchanged
                }
            );
        }
        let count: u64 = GetUnreadCount::new(&Fake(Response::Unread(
            UnreadCountStoreOutcome::Count(u64::MAX),
        )))
        .execute(actor, c)
        .await
        .unwrap();
        assert_eq!(count, u64::MAX);
    });
}
