CREATE TABLE users (
    id BLOB NOT NULL PRIMARY KEY CHECK(length(id) = 16),
    username TEXT NOT NULL UNIQUE,
    disabled_at INTEGER NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;

CREATE TABLE local_credentials (
    user_id BLOB NOT NULL PRIMARY KEY CHECK(length(user_id) = 16),
    password_hash TEXT NOT NULL,
    FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE
) STRICT;

CREATE TABLE sessions (
    id BLOB NOT NULL PRIMARY KEY CHECK(length(id) = 16),
    user_id BLOB NOT NULL CHECK(length(user_id) = 16),
    refresh_token_hash BLOB NOT NULL UNIQUE CHECK(length(refresh_token_hash) = 32),
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    revoked_at INTEGER NULL,
    CHECK(expires_at > created_at),
    FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE
) STRICT;

CREATE INDEX sessions_user_id ON sessions(user_id);
