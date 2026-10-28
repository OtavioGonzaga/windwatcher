CREATE TABLE queue_jobs (
    id BLOB NOT NULL PRIMARY KEY CHECK(length(id) = 16),
    kind TEXT NOT NULL CHECK(length(kind) BETWEEN 1 AND 128),
    payload_version INTEGER NOT NULL CHECK(payload_version >= 1 AND payload_version <= 65535),
    payload_json TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('pending', 'leased', 'completed', 'failed')),
    available_at INTEGER NOT NULL,
    attempts INTEGER NOT NULL CHECK(attempts >= 0),
    max_attempts INTEGER NOT NULL CHECK(max_attempts >= 1),
    lease_id BLOB CHECK(lease_id IS NULL OR length(lease_id) = 16),
    lease_owner BLOB CHECK(lease_owner IS NULL OR length(lease_owner) = 16),
    lease_expires_at INTEGER,
    last_error_code TEXT,
    last_error_message TEXT,
    created_at INTEGER NOT NULL,
    completed_at INTEGER,
    failed_at INTEGER,
    CHECK(attempts <= max_attempts),
    CHECK((status = 'leased' AND lease_id IS NOT NULL AND lease_owner IS NOT NULL AND lease_expires_at IS NOT NULL)
       OR (status != 'leased' AND lease_id IS NULL AND lease_owner IS NULL AND lease_expires_at IS NULL)),
    CHECK((status = 'completed' AND completed_at IS NOT NULL AND failed_at IS NULL)
       OR (status = 'failed' AND failed_at IS NOT NULL AND completed_at IS NULL)
       OR (status IN ('pending', 'leased') AND completed_at IS NULL AND failed_at IS NULL)),
    CHECK((last_error_code IS NULL) = (last_error_message IS NULL)),
    CHECK(last_error_message IS NULL OR length(CAST(last_error_message AS BLOB)) <= 4096)
) STRICT;

CREATE INDEX queue_jobs_pending_claim ON queue_jobs(status, available_at, id);
CREATE INDEX queue_jobs_expired_lease ON queue_jobs(status, lease_expires_at, id);
CREATE INDEX queue_jobs_completed_retention ON queue_jobs(status, completed_at);
CREATE INDEX queue_jobs_operator_listing ON queue_jobs(status, created_at DESC, id DESC);
