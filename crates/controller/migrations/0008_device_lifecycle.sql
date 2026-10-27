ALTER TABLE devices ADD COLUMN state TEXT NOT NULL DEFAULT 'active';
ALTER TABLE devices ADD COLUMN retired_at BIGINT;
ALTER TABLE devices ADD COLUMN superseded_by TEXT;
ALTER TABLE devices ADD COLUMN replaces_device_id TEXT;

CREATE INDEX devices_subject_state_idx
    ON devices(enrolled_by_issuer, enrolled_by_subject, state);
CREATE INDEX devices_superseded_by_idx ON devices(superseded_by);

ALTER TABLE audit_events ADD COLUMN subject TEXT NOT NULL DEFAULT '';

UPDATE audit_events
SET subject = COALESCE(
    (SELECT devices.enrolled_by_subject FROM devices WHERE devices.id = audit_events.device_id),
    ''
)
WHERE subject = '';

CREATE INDEX audit_events_subject_time_idx ON audit_events(subject, created_at);
CREATE INDEX audit_events_action_time_idx ON audit_events(action, created_at);
