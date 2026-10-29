CREATE INDEX conversation_members_joined ON conversation_members(conversation_id,joined_at,user_id);
CREATE TABLE idempotency_records (
    actor_id BINARY(16) NOT NULL,
    operation VARCHAR(32) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    scope_id BINARY(16) NOT NULL,
    key_hash BINARY(32) NOT NULL,
    request_hash BINARY(32) NOT NULL,
    resource_id BINARY(16) NOT NULL,
    created_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    PRIMARY KEY(actor_id,operation,scope_id,key_hash),
    INDEX idempotency_records_expires(expires_at),
    CHECK (operation IN ('create_conversation_v1','send_message_v1')),
    CHECK (expires_at>created_at)
) ENGINE=InnoDB;
