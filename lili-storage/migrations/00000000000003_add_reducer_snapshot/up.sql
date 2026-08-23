UPDATE storage_metadata
SET schema_version = 3
WHERE id = 1;

ALTER TABLE app_state
ADD COLUMN reducer_json TEXT CHECK (reducer_json IS NULL OR json_valid(reducer_json));

UPDATE app_state
SET schema_version = 3
WHERE id = 1;
