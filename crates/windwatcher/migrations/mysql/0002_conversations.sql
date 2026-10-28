CREATE TABLE conversations (
    id BINARY(16) PRIMARY KEY,
    kind VARCHAR(16) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    title LONGBLOB,
    direct_key BINARY(32) UNIQUE,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    CHECK (kind IN ('direct','group')),
    CHECK ((kind='direct' AND title IS NULL AND direct_key IS NOT NULL) OR (kind='group' AND title IS NOT NULL AND direct_key IS NULL))
) ENGINE=InnoDB;
CREATE TABLE conversation_members (
    conversation_id BINARY(16) NOT NULL,
    user_id BINARY(16) NOT NULL,
    role VARCHAR(16) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    joined_at BIGINT NOT NULL,
    owner_conversation_id BINARY(16) GENERATED ALWAYS AS (CASE WHEN role='owner' THEN conversation_id ELSE NULL END) STORED,
    PRIMARY KEY(conversation_id,user_id),
    UNIQUE KEY conversation_members_one_owner(owner_conversation_id),
    INDEX conversation_members_user_id_conversation_id(user_id,conversation_id),
    CONSTRAINT conversation_members_conversation_fk FOREIGN KEY(conversation_id) REFERENCES conversations(id) ON DELETE CASCADE,
    CONSTRAINT conversation_members_user_fk FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE RESTRICT,
    CHECK (role IN ('direct','owner','admin','member'))
) ENGINE=InnoDB;
