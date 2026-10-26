CREATE TABLE audit_events (
    id TEXT PRIMARY KEY,
    actor TEXT NOT NULL,
    action TEXT NOT NULL,
    device_id TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    created_at BIGINT NOT NULL
);

CREATE INDEX audit_events_device_time_idx ON audit_events(device_id, created_at);
