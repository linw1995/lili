CREATE TABLE app_state (
    id INTEGER NOT NULL PRIMARY KEY CHECK (id = 1),
    selected_pet_id TEXT,
    window_placement_json TEXT CHECK (window_placement_json IS NULL OR json_valid(window_placement_json)),
    reducer_revision INTEGER NOT NULL CHECK (reducer_revision >= 0),
    reducer_json TEXT CHECK (reducer_json IS NULL OR json_valid(reducer_json)),
    spool_expired_drops INTEGER NOT NULL DEFAULT 0 CHECK (spool_expired_drops >= 0),
    spool_limit_drops INTEGER NOT NULL DEFAULT 0 CHECK (spool_limit_drops >= 0),
    spool_malformed_drops INTEGER NOT NULL DEFAULT 0 CHECK (spool_malformed_drops >= 0)
);

INSERT INTO app_state (id, reducer_revision)
VALUES (1, 0);

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

CREATE INDEX inbound_spool_claim_idx ON inbound_spool (status, lease_expires_at_ms, priority, occurred_at_ms);
CREATE INDEX inbound_spool_retention_idx ON inbound_spool (status, priority, inserted_at_ms);

CREATE TRIGGER inbound_spool_default_retention
AFTER INSERT ON inbound_spool
BEGIN
    DELETE FROM inbound_spool
    WHERE status = 'pending'
      AND inserted_at_ms < NEW.inserted_at_ms - 86400000;
    UPDATE app_state
    SET spool_expired_drops = spool_expired_drops + changes();

    DELETE FROM inbound_spool
    WHERE rowid IN (
        SELECT rowid
        FROM inbound_spool
        WHERE status = 'pending'
        ORDER BY priority ASC, inserted_at_ms ASC
        LIMIT CASE
            WHEN (SELECT COUNT(*) FROM inbound_spool) > 256
            THEN (SELECT COUNT(*) FROM inbound_spool) - 256
            ELSE 0
        END
    );
    UPDATE app_state
    SET spool_limit_drops = spool_limit_drops + changes();

    DELETE FROM inbound_spool
    WHERE rowid IN (
        SELECT rowid
        FROM inbound_spool
        WHERE status = 'pending'
        ORDER BY priority ASC, inserted_at_ms ASC
        LIMIT CASE
            WHEN (SELECT COALESCE(SUM(length(CAST(payload_json AS BLOB))), 0)
                  FROM inbound_spool) > 4194304
            THEN MAX(
                0,
                (SELECT COUNT(*) FROM inbound_spool WHERE status = 'pending') - 64
            )
            ELSE 0
        END
    );
    UPDATE app_state
    SET spool_limit_drops = spool_limit_drops + changes();
END;
