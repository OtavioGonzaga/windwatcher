CREATE TABLE conversations (
    id BLOB NOT NULL PRIMARY KEY CHECK(length(id) = 16),
    kind TEXT NOT NULL CHECK(kind IN ('direct', 'group')),
    title TEXT NULL,
    direct_key BLOB NULL UNIQUE CHECK(direct_key IS NULL OR length(direct_key) = 32),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    CHECK(
        (kind = 'direct' AND title IS NULL AND direct_key IS NOT NULL)
        OR (kind = 'group' AND title IS NOT NULL AND direct_key IS NULL)
    )
) STRICT;

CREATE TABLE conversation_members (
    conversation_id BLOB NOT NULL CHECK(length(conversation_id) = 16),
    user_id BLOB NOT NULL CHECK(length(user_id) = 16),
    role TEXT NOT NULL CHECK(role IN ('direct', 'owner', 'admin', 'member')),
    joined_at INTEGER NOT NULL,
    PRIMARY KEY(conversation_id, user_id),
    FOREIGN KEY(conversation_id) REFERENCES conversations(id) ON DELETE CASCADE,
    FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE RESTRICT
) STRICT;

CREATE UNIQUE INDEX conversation_members_one_owner
ON conversation_members(conversation_id) WHERE role = 'owner';
CREATE INDEX conversation_members_user_id_conversation_id
ON conversation_members(user_id, conversation_id);
