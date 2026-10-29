CREATE TABLE messages (
    id BINARY(16) PRIMARY KEY,
    conversation_id BINARY(16) NOT NULL,
    sender_id BINARY(16) NOT NULL,
    content LONGTEXT CHARACTER SET utf8mb4 COLLATE utf8mb4_bin,
    reply_to_message_id BINARY(16),
    created_at BIGINT NOT NULL,
    edited_at BIGINT,
    deleted_at BIGINT,
    UNIQUE KEY messages_conversation_id_id(conversation_id,id),
    CONSTRAINT messages_conversation_fk FOREIGN KEY(conversation_id) REFERENCES conversations(id) ON DELETE CASCADE,
    CONSTRAINT messages_sender_fk FOREIGN KEY(sender_id) REFERENCES users(id) ON DELETE RESTRICT,
    CONSTRAINT messages_reply_fk FOREIGN KEY(conversation_id,reply_to_message_id) REFERENCES messages(conversation_id,id) ON DELETE NO ACTION,
    CHECK ((deleted_at IS NULL AND content IS NOT NULL) OR (deleted_at IS NOT NULL AND content IS NULL)),
    CHECK (reply_to_message_id IS NULL OR reply_to_message_id <> id)
) ENGINE=InnoDB;
ALTER TABLE conversation_members ADD COLUMN last_read_message_id BINARY(16);
