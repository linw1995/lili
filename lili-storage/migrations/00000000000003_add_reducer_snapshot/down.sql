ALTER TABLE app_state DROP COLUMN reducer_json;

UPDATE app_state
SET schema_version = 2
WHERE id = 1;

UPDATE storage_metadata
SET schema_version = 2
WHERE id = 1;
