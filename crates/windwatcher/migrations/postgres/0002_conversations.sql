CREATE TABLE conversations (
    id UUID PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('direct','group')),
    title BYTEA,
    direct_key BYTEA UNIQUE CHECK (direct_key IS NULL OR octet_length(direct_key)=32),
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    CHECK ((kind='direct' AND title IS NULL AND direct_key IS NOT NULL) OR
           (kind='group' AND title IS NOT NULL AND direct_key IS NULL))
);
CREATE TABLE conversation_members (
    conversation_id UUID NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    role TEXT NOT NULL CHECK (role IN ('direct','owner','admin','member')),
    joined_at BIGINT NOT NULL,
    PRIMARY KEY (conversation_id,user_id)
);
CREATE UNIQUE INDEX conversation_members_one_owner ON conversation_members(conversation_id) WHERE role='owner';
CREATE INDEX conversation_members_user_id_conversation_id ON conversation_members(user_id,conversation_id);
