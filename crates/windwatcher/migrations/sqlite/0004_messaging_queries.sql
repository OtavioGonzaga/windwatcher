CREATE INDEX conversation_members_joined
    ON conversation_members(conversation_id, joined_at, user_id);

CREATE TABLE idempotency_records (
    actor_id BLOB NOT NULL CHECK(length(actor_id) = 16),
    operation TEXT NOT NULL CHECK(operation IN ('create_conversation_v1', 'send_message_v1')),
    scope_id BLOB NOT NULL CHECK(length(scope_id) = 16),
    key_hash BLOB NOT NULL CHECK(length(key_hash) = 32),
    request_hash BLOB NOT NULL CHECK(length(request_hash) = 32),
    resource_id BLOB NOT NULL CHECK(length(resource_id) = 16),
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL CHECK(expires_at > created_at),
    PRIMARY KEY(actor_id, operation, scope_id, key_hash)
) STRICT;

CREATE INDEX idempotency_records_expires ON idempotency_records(expires_at);
