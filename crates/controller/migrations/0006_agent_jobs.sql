CREATE TABLE agent_jobs (
    id TEXT PRIMARY KEY,
    device_id TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    attempt BIGINT NOT NULL DEFAULT 1,
    idempotency_key TEXT NOT NULL,
    origin TEXT NOT NULL,
    requested_by TEXT NOT NULL,
    catalog_id TEXT NOT NULL,
    catalog_revision BIGINT NOT NULL,
    catalog_sha256 TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    version TEXT NOT NULL,
    action BIGINT NOT NULL,
    state BIGINT NOT NULL,
    deadline_unix_seconds BIGINT NOT NULL,
    error_code TEXT NOT NULL DEFAULT '',
    error_message TEXT NOT NULL DEFAULT '',
    diagnostics_json TEXT NOT NULL DEFAULT '',
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    UNIQUE (device_id, idempotency_key)
);

CREATE INDEX agent_jobs_device_state_idx ON agent_jobs(device_id, state);

CREATE TABLE agent_install_receipts (
    device_id TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    agent_id TEXT NOT NULL,
    version TEXT NOT NULL,
    receipt_json TEXT NOT NULL,
    updated_at BIGINT NOT NULL,
    PRIMARY KEY (device_id, agent_id)
);
