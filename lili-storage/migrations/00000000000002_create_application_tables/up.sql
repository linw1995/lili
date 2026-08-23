UPDATE storage_metadata
SET schema_version = 2
WHERE id = 1;

CREATE TABLE app_state (
    id INTEGER NOT NULL PRIMARY KEY CHECK (id = 1),
    schema_version INTEGER NOT NULL CHECK (schema_version > 0),
    selected_pet_id TEXT,
    window_placement_json TEXT CHECK (window_placement_json IS NULL OR json_valid(window_placement_json)),
    reducer_revision INTEGER NOT NULL CHECK (reducer_revision >= 0),
    presentation_state TEXT NOT NULL CHECK (presentation_state IN ('idle', 'running', 'review', 'failed', 'waiting')),
    presentation_since_ms INTEGER NOT NULL CHECK (presentation_since_ms >= 0),
    minimum_dwell_ms INTEGER NOT NULL CHECK (minimum_dwell_ms >= 0)
);

INSERT INTO app_state (
    id,
    schema_version,
    reducer_revision,
    presentation_state,
    presentation_since_ms,
    minimum_dwell_ms
)
VALUES (1, 2, 0, 'idle', 0, 750);

CREATE TABLE sessions (
    provider TEXT NOT NULL CHECK (length(provider) BETWEEN 1 AND 256),
    session_id TEXT NOT NULL CHECK (length(session_id) BETWEEN 1 AND 256),
    current_turn_id TEXT,
    project_json TEXT CHECK (project_json IS NULL OR json_valid(project_json)),
    updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms >= 0),
    last_event_id TEXT NOT NULL CHECK (length(last_event_id) BETWEEN 1 AND 256),
    ended INTEGER NOT NULL CHECK (ended IN (0, 1)),
    PRIMARY KEY (provider, session_id)
);

CREATE TABLE turns (
    provider TEXT NOT NULL CHECK (length(provider) BETWEEN 1 AND 256),
    session_id TEXT NOT NULL CHECK (length(session_id) BETWEEN 1 AND 256),
    turn_id TEXT NOT NULL CHECK (length(turn_id) BETWEEN 1 AND 256),
    phase TEXT NOT NULL CHECK (phase IN ('idle', 'active', 'attention', 'completed', 'failed', 'ended')),
    updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms >= 0),
    last_event_id TEXT NOT NULL CHECK (length(last_event_id) BETWEEN 1 AND 256),
    PRIMARY KEY (provider, session_id, turn_id),
    FOREIGN KEY (provider, session_id) REFERENCES sessions(provider, session_id) ON DELETE CASCADE
);

CREATE TABLE notifications (
    id TEXT NOT NULL PRIMARY KEY CHECK (length(id) BETWEEN 1 AND 256),
    provider TEXT NOT NULL CHECK (length(provider) BETWEEN 1 AND 256),
    event_id TEXT NOT NULL CHECK (length(event_id) BETWEEN 1 AND 256),
    session_id TEXT NOT NULL CHECK (length(session_id) BETWEEN 1 AND 256),
    turn_id TEXT,
    kind TEXT NOT NULL CHECK (kind IN ('attention', 'completion', 'failure')),
    state TEXT NOT NULL CHECK (state IN ('unread', 'acknowledged', 'resolved')),
    occurred_at_ms INTEGER NOT NULL CHECK (occurred_at_ms >= 0),
    project_json TEXT CHECK (project_json IS NULL OR json_valid(project_json)),
    summary_json TEXT CHECK (summary_json IS NULL OR json_valid(summary_json)),
    FOREIGN KEY (provider, session_id) REFERENCES sessions(provider, session_id) ON DELETE CASCADE
);

CREATE TABLE recent_events (
    provider TEXT NOT NULL CHECK (length(provider) BETWEEN 1 AND 256),
    event_id TEXT NOT NULL CHECK (length(event_id) BETWEEN 1 AND 256),
    observed_at_ms INTEGER NOT NULL CHECK (observed_at_ms >= 0),
    PRIMARY KEY (provider, event_id)
);

CREATE TABLE inbound_spool (
    provider TEXT NOT NULL CHECK (length(provider) BETWEEN 1 AND 256),
    event_id TEXT NOT NULL CHECK (length(event_id) BETWEEN 1 AND 256),
    payload_json TEXT NOT NULL CHECK (json_valid(payload_json)),
    priority INTEGER NOT NULL CHECK (priority BETWEEN 0 AND 255),
    occurred_at_ms INTEGER NOT NULL CHECK (occurred_at_ms >= 0),
    inserted_at_ms INTEGER NOT NULL CHECK (inserted_at_ms >= 0),
    status TEXT NOT NULL CHECK (status IN ('pending', 'claimed')),
    claim_token TEXT,
    claimed_at_ms INTEGER,
    lease_expires_at_ms INTEGER,
    attempts INTEGER NOT NULL CHECK (attempts >= 0),
    PRIMARY KEY (provider, event_id)
);

CREATE TABLE plugin_evidence (
    id INTEGER NOT NULL PRIMARY KEY CHECK (id = 1),
    evidence_json TEXT NOT NULL CHECK (json_valid(evidence_json)),
    updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms >= 0)
);

CREATE TABLE lifecycle_events (
    event_id TEXT NOT NULL PRIMARY KEY CHECK (length(event_id) BETWEEN 1 AND 256),
    entity_type TEXT NOT NULL CHECK (entity_type IN ('application', 'session', 'turn', 'notification', 'spool')),
    entity_id TEXT NOT NULL CHECK (length(entity_id) BETWEEN 1 AND 256),
    event_type TEXT NOT NULL CHECK (length(event_type) BETWEEN 1 AND 128),
    source TEXT NOT NULL CHECK (length(source) BETWEEN 1 AND 128),
    occurred_at_ms INTEGER NOT NULL CHECK (occurred_at_ms >= 0),
    previous_state TEXT,
    current_state TEXT,
    details_json TEXT CHECK (details_json IS NULL OR json_valid(details_json)),
    error_json TEXT CHECK (error_json IS NULL OR json_valid(error_json))
);

CREATE INDEX sessions_updated_idx ON sessions (updated_at_ms);
CREATE INDEX turns_session_updated_idx ON turns (provider, session_id, updated_at_ms);
CREATE INDEX notifications_session_time_idx ON notifications (provider, session_id, occurred_at_ms);
CREATE INDEX recent_events_observed_idx ON recent_events (observed_at_ms);
CREATE INDEX inbound_spool_claim_idx ON inbound_spool (status, lease_expires_at_ms, priority, occurred_at_ms);
CREATE INDEX lifecycle_events_entity_time_idx ON lifecycle_events (entity_type, entity_id, occurred_at_ms);

CREATE TRIGGER lifecycle_events_immutable_update
BEFORE UPDATE ON lifecycle_events
BEGIN
    SELECT RAISE(ABORT, 'lifecycle_events are immutable');
END;

CREATE TRIGGER lifecycle_events_immutable_delete
BEFORE DELETE ON lifecycle_events
BEGIN
    SELECT RAISE(ABORT, 'lifecycle_events are immutable');
END;
