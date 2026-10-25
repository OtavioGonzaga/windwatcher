CREATE TABLE messages (
    id BLOB NOT NULL PRIMARY KEY CHECK(length(id) = 16),
    conversation_id BLOB NOT NULL CHECK(length(conversation_id) = 16),
    sender_id BLOB NOT NULL CHECK(length(sender_id) = 16),
    content TEXT NULL,
    reply_to_message_id BLOB NULL CHECK(reply_to_message_id IS NULL OR length(reply_to_message_id) = 16),
    created_at INTEGER NOT NULL,
    edited_at INTEGER NULL,
    deleted_at INTEGER NULL,
    UNIQUE(conversation_id, id),
    CHECK((deleted_at IS NULL AND content IS NOT NULL) OR (deleted_at IS NOT NULL AND content IS NULL)),
    CHECK(reply_to_message_id IS NULL OR reply_to_message_id <> id),
    FOREIGN KEY(conversation_id) REFERENCES conversations(id) ON DELETE CASCADE,
    FOREIGN KEY(sender_id) REFERENCES users(id) ON DELETE RESTRICT,
    FOREIGN KEY(conversation_id, reply_to_message_id) REFERENCES messages(conversation_id, id) ON DELETE NO ACTION
) STRICT;

ALTER TABLE conversation_members ADD COLUMN last_read_message_id BLOB NULL
    CHECK(last_read_message_id IS NULL OR length(last_read_message_id) = 16);
