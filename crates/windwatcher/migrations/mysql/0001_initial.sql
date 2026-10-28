CREATE TABLE users (
    id BINARY(16) PRIMARY KEY,
    username VARCHAR(32) CHARACTER SET ascii COLLATE ascii_bin NOT NULL UNIQUE,
    disabled_at BIGINT,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    CHECK (username REGEXP '^[a-z0-9_]{3,32}$')
) ENGINE=InnoDB;
CREATE TABLE local_credentials (
    user_id BINARY(16) PRIMARY KEY,
    password_hash TEXT NOT NULL,
    CONSTRAINT local_credentials_user_fk FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE
) ENGINE=InnoDB;
CREATE TABLE sessions (
    id BINARY(16) PRIMARY KEY,
    user_id BINARY(16) NOT NULL,
    refresh_token_hash BINARY(32) NOT NULL UNIQUE,
    created_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    revoked_at BIGINT,
    CHECK (expires_at > created_at),
    CONSTRAINT sessions_user_fk FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE,
    INDEX sessions_user_id(user_id)
) ENGINE=InnoDB;
