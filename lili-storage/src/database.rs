use std::fmt;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use diesel::Connection;
use diesel::connection::SimpleConnection;
use diesel::sqlite::SqliteConnection;
use diesel_migrations::{EmbeddedMigrations, MigrationHarness, embed_migrations};

use crate::{ApplicationPaths, PathError, StorageError};

pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!();

const CONNECTION_PRAGMAS: &str =
    "PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000; PRAGMA journal_mode = WAL;";
const INITIALIZATION_RETRY_COUNT: usize = 100;
const INITIALIZATION_RETRY_DELAY: Duration = Duration::from_millis(25);

pub struct EmbeddedDatabase {
    connection: SqliteConnection,
}

impl EmbeddedDatabase {
    pub fn connection(&mut self) -> &mut SqliteConnection {
        &mut self.connection
    }
}

pub fn open(paths: &ApplicationPaths) -> Result<EmbeddedDatabase, DatabaseError> {
    paths.ensure_layout().map_err(DatabaseError::Storage)?;
    let connection = connect(&paths.database_path())?;
    paths.ensure_layout().map_err(DatabaseError::Storage)?;
    Ok(EmbeddedDatabase { connection })
}

pub fn connect(path: &Path) -> Result<SqliteConnection, DatabaseError> {
    let mut last_error = None;
    for attempt in 0..=INITIALIZATION_RETRY_COUNT {
        match connect_once(path) {
            Ok(connection) => return Ok(connection),
            Err(error) if error.is_retryable() && attempt < INITIALIZATION_RETRY_COUNT => {
                last_error = Some(error);
                thread::sleep(INITIALIZATION_RETRY_DELAY);
            }
            Err(error) => return Err(error),
        }
    }
    Err(last_error.expect("database initialization must produce an error"))
}

fn connect_once(path: &Path) -> Result<SqliteConnection, DatabaseError> {
    let path_text = path
        .to_str()
        .ok_or_else(|| DatabaseError::PathNotUtf8(path.to_owned()))?;
    let mut connection =
        SqliteConnection::establish(path_text).map_err(DatabaseError::Connection)?;
    connection
        .batch_execute(CONNECTION_PRAGMAS)
        .map_err(DatabaseError::Configuration)?;
    connection
        .run_pending_migrations(MIGRATIONS)
        .map_err(DatabaseError::Migration)?;
    Ok(connection)
}

#[derive(Debug)]
pub enum DatabaseError {
    Path(PathError),
    Storage(StorageError),
    PathNotUtf8(PathBuf),
    Connection(diesel::ConnectionError),
    Configuration(diesel::result::Error),
    Migration(Box<dyn std::error::Error + Send + Sync>),
}

impl fmt::Display for DatabaseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Path(error) => error.fmt(formatter),
            Self::Storage(error) => error.fmt(formatter),
            Self::PathNotUtf8(path) => {
                write!(
                    formatter,
                    "database path is not valid UTF-8: {}",
                    path.display()
                )
            }
            Self::Connection(error) => write!(formatter, "failed to open SQLite database: {error}"),
            Self::Configuration(error) => {
                write!(formatter, "failed to configure SQLite connection: {error}")
            }
            Self::Migration(error) => {
                write!(formatter, "failed to apply database migrations: {error}")
            }
        }
    }
}

impl std::error::Error for DatabaseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Path(error) => Some(error),
            Self::Storage(error) => Some(error),
            Self::PathNotUtf8(_) => None,
            Self::Connection(error) => Some(error),
            Self::Configuration(error) => Some(error),
            Self::Migration(error) => Some(&**error),
        }
    }
}

impl DatabaseError {
    fn is_retryable(&self) -> bool {
        match self {
            Self::Configuration(error) => error.to_string().contains("database is locked"),
            Self::Migration(error) => {
                let message = error.to_string();
                message.contains("database is locked") || message.contains("already exists")
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use diesel::prelude::*;
    use diesel_migrations::MigrationHarness;

    use super::*;
    use crate::models::NewLifecycleEvent;
    use crate::repository::insert_lifecycle_event;
    use crate::schema::lifecycle_events;

    #[derive(QueryableByName)]
    struct CountRow {
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        count: i64,
    }

    fn temporary_paths() -> ApplicationPaths {
        let root =
            std::env::temp_dir().join(format!("lili-storage-database-{}", uuid::Uuid::new_v4()));
        ApplicationPaths::from_root(root).unwrap()
    }

    #[test]
    fn open_creates_database_and_applies_metadata_migration() {
        let paths = temporary_paths();
        let database_path = paths.database_path();
        let mut database = open(&paths).unwrap();
        let count = diesel::sql_query("SELECT COUNT(*) AS count FROM storage_metadata")
            .get_result::<CountRow>(database.connection())
            .unwrap()
            .count;

        assert_eq!(count, 1);
        assert!(database_path.is_file());
        drop(database);
        fs::remove_dir_all(paths.root()).unwrap();
    }

    #[test]
    fn connection_enables_foreign_keys_and_wal() {
        let paths = temporary_paths();
        let mut database = open(&paths).unwrap();
        let foreign_keys = diesel::sql_query("PRAGMA foreign_keys")
            .get_result::<ForeignKeysRow>(database.connection())
            .unwrap()
            .foreign_keys;
        let journal_mode = diesel::sql_query("PRAGMA journal_mode")
            .get_result::<JournalModeRow>(database.connection())
            .unwrap()
            .journal_mode;

        assert_eq!(foreign_keys, 1);
        assert_eq!(journal_mode, "wal");
        drop(database);
        fs::remove_dir_all(paths.root()).unwrap();
    }

    #[test]
    fn embedded_migrations_can_be_reverted_and_rerun() {
        let paths = temporary_paths();
        let mut database = open(&paths).unwrap();
        database
            .connection()
            .revert_last_migration(MIGRATIONS)
            .unwrap();
        database
            .connection()
            .run_pending_migrations(MIGRATIONS)
            .unwrap();

        let count = diesel::sql_query("SELECT COUNT(*) AS count FROM app_state")
            .get_result::<CountRow>(database.connection())
            .unwrap()
            .count;
        assert_eq!(count, 1);
        drop(database);
        fs::remove_dir_all(paths.root()).unwrap();
    }

    #[test]
    fn schema_rejects_invalid_json_and_orphaned_notifications() {
        let paths = temporary_paths();
        let mut database = open(&paths).unwrap();
        let invalid_json = diesel::sql_query(
            "INSERT INTO inbound_spool (provider, event_id, payload_json, priority, occurred_at_ms, inserted_at_ms, status, attempts) VALUES ('codex', 'event-1', 'not-json', 1, 1, 1, 'pending', 0)",
        )
        .execute(database.connection());
        assert!(invalid_json.is_err());

        let orphaned_notification = diesel::sql_query(
            "INSERT INTO notifications (id, provider, event_id, session_id, kind, state, occurred_at_ms) VALUES ('notification-1', 'codex', 'event-1', 'missing-session', 'completion', 'unread', 1)",
        )
        .execute(database.connection());
        assert!(orphaned_notification.is_err());
        drop(database);
        fs::remove_dir_all(paths.root()).unwrap();
    }

    #[test]
    fn lifecycle_events_are_immutable() {
        let paths = temporary_paths();
        let mut database = open(&paths).unwrap();
        insert_lifecycle_event(
            database.connection(),
            &NewLifecycleEvent {
                event_id: "event-1",
                entity_type: "application",
                entity_id: "app",
                event_type: "started",
                source: "test",
                occurred_at_ms: 1,
                previous_state: None,
                current_state: Some("idle"),
                details_json: None,
                error_json: None,
            },
        )
        .unwrap();

        let update = diesel::update(lifecycle_events::table.find("event-1"))
            .set(lifecycle_events::event_type.eq("changed"))
            .execute(database.connection());
        let delete =
            diesel::delete(lifecycle_events::table.find("event-1")).execute(database.connection());
        assert!(update.is_err());
        assert!(delete.is_err());
        drop(database);
        fs::remove_dir_all(paths.root()).unwrap();
    }

    #[derive(QueryableByName)]
    struct JournalModeRow {
        #[diesel(sql_type = diesel::sql_types::Text)]
        journal_mode: String,
    }

    #[derive(QueryableByName)]
    struct ForeignKeysRow {
        #[diesel(sql_type = diesel::sql_types::Integer)]
        foreign_keys: i32,
    }
}
