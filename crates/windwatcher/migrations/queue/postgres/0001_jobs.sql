CREATE TABLE queue_jobs (
    id UUID PRIMARY KEY,
    kind TEXT NOT NULL CHECK (length(kind) BETWEEN 1 AND 128),
    payload_version INTEGER NOT NULL CHECK (payload_version BETWEEN 1 AND 65535),
    payload_json JSONB NOT NULL CHECK (jsonb_typeof(payload_json) = 'object'),
    status TEXT NOT NULL CHECK (status IN ('pending','leased','completed','failed')),
    available_at TIMESTAMPTZ NOT NULL,
    attempts BIGINT NOT NULL CHECK (attempts >= 0),
    max_attempts BIGINT NOT NULL CHECK (max_attempts BETWEEN 1 AND 4294967295),
    lease_id UUID,
    lease_owner UUID,
    lease_expires_at TIMESTAMPTZ,
    last_error_code TEXT,
    last_error_message TEXT CHECK (octet_length(last_error_message) <= 4096),
    created_at TIMESTAMPTZ NOT NULL,
    completed_at TIMESTAMPTZ,
    failed_at TIMESTAMPTZ,
    CHECK (attempts <= max_attempts),
    CHECK ((status = 'pending' AND attempts < max_attempts) OR (status != 'pending' AND attempts >= 1)),
    CHECK ((status = 'leased' AND lease_id IS NOT NULL AND lease_owner IS NOT NULL AND lease_expires_at IS NOT NULL)
        OR (status != 'leased' AND lease_id IS NULL AND lease_owner IS NULL AND lease_expires_at IS NULL)),
    CHECK ((status = 'completed' AND completed_at IS NOT NULL AND failed_at IS NULL)
        OR (status = 'failed' AND failed_at IS NOT NULL AND completed_at IS NULL)
        OR (status IN ('pending','leased') AND completed_at IS NULL AND failed_at IS NULL)),
    CHECK ((last_error_code IS NULL) = (last_error_message IS NULL)),
    CHECK (status != 'completed' OR last_error_code IS NULL),
    CHECK (status != 'failed' OR last_error_code IS NOT NULL)
);
CREATE INDEX queue_jobs_pending_claim ON queue_jobs(status, available_at, id);
CREATE INDEX queue_jobs_expired_lease ON queue_jobs(status, lease_expires_at, id);
CREATE INDEX queue_jobs_completed_retention ON queue_jobs(status, completed_at);
CREATE INDEX queue_jobs_operator_listing ON queue_jobs(status, created_at DESC, id DESC);
