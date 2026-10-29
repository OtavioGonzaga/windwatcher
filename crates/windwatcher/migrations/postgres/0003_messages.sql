CREATE TABLE messages (
    id UUID PRIMARY KEY,
    conversation_id UUID NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    sender_id UUID NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    content TEXT,
    reply_to_message_id UUID,
    created_at BIGINT NOT NULL,
    edited_at BIGINT,
    deleted_at BIGINT,
    UNIQUE (conversation_id,id),
    CHECK ((deleted_at IS NULL AND content IS NOT NULL) OR (deleted_at IS NOT NULL AND content IS NULL)),
    CHECK (reply_to_message_id IS NULL OR reply_to_message_id <> id),
    FOREIGN KEY (conversation_id,reply_to_message_id) REFERENCES messages(conversation_id,id) ON DELETE NO ACTION
);
ALTER TABLE conversation_members ADD COLUMN last_read_message_id UUID;
