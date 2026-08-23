CREATE TABLE storage_metadata (
    id INTEGER NOT NULL PRIMARY KEY CHECK (id = 1),
    schema_version INTEGER NOT NULL CHECK (schema_version > 0)
);

INSERT INTO storage_metadata (id, schema_version)
VALUES (1, 1);
