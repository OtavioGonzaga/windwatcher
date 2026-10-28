CREATE TABLE queue_jobs (
    id BINARY(16) NOT NULL PRIMARY KEY,
    kind VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    payload_version INTEGER NOT NULL CHECK (payload_version BETWEEN 1 AND 65535),
    payload_json MEDIUMBLOB NOT NULL,
    status VARCHAR(16) CHARACTER SET ascii COLLATE ascii_bin NOT NULL CHECK (status IN ('pending','leased','completed','failed')),
    available_at BIGINT NOT NULL,
    attempts BIGINT NOT NULL CHECK (attempts >= 0),
    max_attempts BIGINT NOT NULL CHECK (max_attempts BETWEEN 1 AND 4294967295),
    lease_id BINARY(16),
    lease_owner BINARY(16),
    lease_expires_at BIGINT,
    last_error_code VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin,
    last_error_message BLOB CHECK (OCTET_LENGTH(last_error_message) <= 4096),
    created_at BIGINT NOT NULL,
    completed_at BIGINT,
    failed_at BIGINT,
    CHECK (attempts <= max_attempts),
    CHECK ((status = 'pending' AND attempts < max_attempts) OR (status != 'pending' AND attempts >= 1)),
    CHECK ((status = 'leased' AND lease_id IS NOT NULL AND lease_owner IS NOT NULL AND lease_expires_at IS NOT NULL)
        OR (status != 'leased' AND lease_id IS NULL AND lease_owner IS NULL AND lease_expires_at IS NULL)),
    CHECK ((status = 'completed' AND completed_at IS NOT NULL AND failed_at IS NULL)
        OR (status = 'failed' AND failed_at IS NOT NULL AND completed_at IS NULL)
        OR (status IN ('pending','leased') AND completed_at IS NULL AND failed_at IS NULL)),
    CHECK ((last_error_code IS NULL) = (last_error_message IS NULL)),
    CHECK (status != 'completed' OR last_error_code IS NULL),
    CHECK (status != 'failed' OR last_error_code IS NOT NULL),
    INDEX queue_jobs_pending_claim (status,available_at,id),
    INDEX queue_jobs_expired_lease (status,lease_expires_at,id),
    INDEX queue_jobs_completed_retention (status,completed_at),
    INDEX queue_jobs_operator_listing (status,created_at,id)
) ENGINE=InnoDB;
