CREATE INDEX conversation_members_joined
    ON conversation_members(conversation_id, joined_at, user_id);
