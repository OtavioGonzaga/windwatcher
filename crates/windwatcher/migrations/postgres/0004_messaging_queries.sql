CREATE INDEX conversation_members_joined ON conversation_members(conversation_id,joined_at,user_id);
CREATE INDEX messages_conversation_id_id ON messages(conversation_id,id);
CREATE TABLE idempotency_records (
    actor_id UUID NOT NULL,
    operation TEXT NOT NULL CHECK (operation IN ('create_conversation_v1','send_message_v1')),
    scope_id UUID NOT NULL,
    key_hash BYTEA NOT NULL CHECK (octet_length(key_hash)=32),
    request_hash BYTEA NOT NULL CHECK (octet_length(request_hash)=32),
    resource_id UUID NOT NULL,
    created_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL CHECK (expires_at>created_at),
    PRIMARY KEY(actor_id,operation,scope_id,key_hash)
);
CREATE INDEX idempotency_records_expires ON idempotency_records(expires_at);
