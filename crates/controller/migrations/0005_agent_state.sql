CREATE TABLE device_agent_state (
    device_id TEXT PRIMARY KEY REFERENCES devices(id) ON DELETE CASCADE,
    catalog_revision BIGINT NOT NULL,
    catalog_sha256 TEXT NOT NULL,
    last_sync_at BIGINT NOT NULL,
    stale BIGINT NOT NULL DEFAULT 0,
    agents_json TEXT NOT NULL DEFAULT '[]',
    updated_at BIGINT NOT NULL
);
