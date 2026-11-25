-- These tables belong to the host app's migration.
-- A router role needs SELECT and UPDATE on intake_items, SELECT and INSERT on
-- intake_events, and SELECT on intake_attachments. It needs no sequence privilege.
-- A read-only role needs SELECT on all three tables.
CREATE TABLE intake_items (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    source text NOT NULL,
    requester text NOT NULL,
    status text NOT NULL,
    title text NOT NULL,
    body text NOT NULL,
    snapshot json NULL,
    triage json NULL,
    proposal json NULL,
    created_at_ms bigint NOT NULL,
    updated_at_ms bigint NOT NULL,
    event_count integer NOT NULL,
    CHECK (event_count BETWEEN 1 AND 256),
    CHECK (updated_at_ms >= created_at_ms)
);
CREATE INDEX intake_items_status_id ON intake_items (status, id);
CREATE TABLE intake_events (
    item_id bigint NOT NULL REFERENCES intake_items (id),
    seq integer NOT NULL,
    event json NOT NULL,
    PRIMARY KEY (item_id, seq),
    CHECK (seq BETWEEN 0 AND 255)
);
CREATE TABLE intake_attachments (
    item_id bigint NOT NULL REFERENCES intake_items (id),
    file_index integer NOT NULL,
    meta json NOT NULL,
    bytes bytea NOT NULL,
    PRIMARY KEY (item_id, file_index),
    CHECK (file_index >= 0)
);
