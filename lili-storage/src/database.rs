use std::fmt;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use diesel::Connection;
use diesel::connection::SimpleConnection;
use diesel::sqlite::SqliteConnection;
use diesel_migrations::{EmbeddedMigrations, MigrationHarness, embed_migrations};

use crate::{ApplicationPaths, PathError, StorageError};

pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!();

const DEFAULT_BUSY_TIMEOUT: Duration = Duration::from_millis(5_000);
const INITIALIZATION_RETRY_DELAY: Duration = Duration::from_millis(25);
const DEFAULT_INITIALIZATION_DEADLINE: Duration = Duration::from_secs(15);

pub struct EmbeddedDatabase {
    connection: SqliteConnection,
}

impl EmbeddedDatabase {
    pub fn connection(&mut self) -> &mut SqliteConnection {
        &mut self.connection
    }
}

pub fn open(paths: &ApplicationPaths) -> Result<EmbeddedDatabase, DatabaseError> {
    open_with_options(paths, DEFAULT_BUSY_TIMEOUT, DEFAULT_INITIALIZATION_DEADLINE)
}

pub fn open_with_busy_timeout(
    paths: &ApplicationPaths,
    busy_timeout: Duration,
) -> Result<EmbeddedDatabase, DatabaseError> {
    let initialization_deadline = busy_timeout + INITIALIZATION_RETRY_DELAY;
    open_with_options(paths, busy_timeout, initialization_deadline)
}

fn open_with_options(
    paths: &ApplicationPaths,
    busy_timeout: Duration,
    initialization_deadline: Duration,
) -> Result<EmbeddedDatabase, DatabaseError> {
    paths.ensure_layout().map_err(DatabaseError::Storage)?;
    let connection = connect_with_options(
        &paths.database_path(),
        busy_timeout,
        initialization_deadline,
    )?;
    paths.ensure_layout().map_err(DatabaseError::Storage)?;
    Ok(EmbeddedDatabase { connection })
}

pub fn connect(path: &Path) -> Result<SqliteConnection, DatabaseError> {
    connect_with_options(path, DEFAULT_BUSY_TIMEOUT, DEFAULT_INITIALIZATION_DEADLINE)
}

fn connect_with_options(
    path: &Path,
    busy_timeout: Duration,
    initialization_deadline: Duration,
) -> Result<SqliteConnection, DatabaseError> {
    let deadline = Instant::now() + initialization_deadline;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let attempt_timeout = busy_timeout.min(remaining);
        match connect_once(path, attempt_timeout) {
            Ok(connection) => return Ok(connection),
            Err(error) if error.is_retryable() => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(error);
                }
                thread::sleep(INITIALIZATION_RETRY_DELAY.min(remaining));
            }
            Err(error) => return Err(error),
        }
    }
}

fn connect_once(path: &Path, busy_timeout: Duration) -> Result<SqliteConnection, DatabaseError> {
    let path_text = path
        .to_str()
        .ok_or_else(|| DatabaseError::PathNotUtf8(path.to_owned()))?;
    let mut connection =
        SqliteConnection::establish(path_text).map_err(DatabaseError::Connection)?;
    let busy_timeout_ms = busy_timeout.as_millis().min(i64::MAX as u128);
    connection
        .batch_execute(&format!(
            "PRAGMA foreign_keys = ON; PRAGMA busy_timeout = {busy_timeout_ms}; PRAGMA journal_mode = WAL;"
        ))
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
    use crate::models::AppStateRow;
    use crate::repository::{load_app_state, update_app_state, update_app_state_if_newer};

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
    fn database_errors_expose_bounded_display_and_sources() {
        use std::error::Error;

        let errors = [
            DatabaseError::Path(PathError::HomeDirectoryUnavailable),
            DatabaseError::Storage(StorageError::InvalidDirectory(PathBuf::from("/tmp/root"))),
            DatabaseError::Connection(diesel::ConnectionError::InvalidConnectionUrl(
                "invalid".to_owned(),
            )),
            DatabaseError::Configuration(diesel::result::Error::RollbackTransaction),
            DatabaseError::Migration(Box::new(std::io::Error::other("migration failed"))),
        ];
        for error in errors {
            assert!(!error.to_string().is_empty());
            assert!(error.source().is_some());
        }

        let path_error = DatabaseError::PathNotUtf8(PathBuf::from("/tmp/database"));
        assert!(path_error.to_string().contains("database path"));
        assert!(path_error.source().is_none());
    }

    #[test]
    fn open_creates_database_and_applies_initial_migration() {
        let paths = temporary_paths();
        let database_path = paths.database_path();
        let mut database = open(&paths).unwrap();
        let app_state_count = diesel::sql_query("SELECT COUNT(*) AS count FROM app_state")
            .get_result::<CountRow>(database.connection())
            .unwrap()
            .count;
        let migration_count =
            diesel::sql_query("SELECT COUNT(*) AS count FROM __diesel_schema_migrations")
                .get_result::<CountRow>(database.connection())
                .unwrap()
                .count;

        assert_eq!(app_state_count, 1);
        assert_eq!(migration_count, 1);
        assert!(database_path.is_file());
        drop(database);
        fs::remove_dir_all(paths.root()).unwrap();
    }

    #[test]
    fn stale_application_state_update_cannot_replace_a_newer_revision() {
        let paths = temporary_paths();
        let mut database = open(&paths).unwrap();
        let current = AppStateRow {
            id: 1,
            selected_pet_id: Some("new-pet".to_owned()),
            window_placement_json: None,
            reducer_json: Some(r#"{"revision":2}"#.to_owned()),
            reducer_revision: 2,
            spool_expired_drops: 0,
            spool_limit_drops: 0,
            spool_malformed_drops: 0,
        };
        update_app_state(database.connection(), &current).unwrap();

        let stale = AppStateRow {
            id: 1,
            selected_pet_id: Some("old-pet".to_owned()),
            window_placement_json: None,
            reducer_json: Some(r#"{"revision":1}"#.to_owned()),
            reducer_revision: 1,
            spool_expired_drops: 0,
            spool_limit_drops: 0,
            spool_malformed_drops: 0,
        };
        assert!(!update_app_state_if_newer(database.connection(), &stale).unwrap());

        let persisted = load_app_state(database.connection()).unwrap();
        assert_eq!(persisted.reducer_revision, 2);
        assert_eq!(persisted.selected_pet_id.as_deref(), Some("new-pet"));
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
    fn schema_rejects_invalid_json() {
        let paths = temporary_paths();
        let mut database = open(&paths).unwrap();
        let invalid_json = diesel::sql_query(
            "INSERT INTO inbound_spool (provider, event_id, payload_json, priority, occurred_at_ms, inserted_at_ms, status, attempts) VALUES ('codex', 'event-1', 'not-json', 1, 1, 1, 'pending', 0)",
        )
        .execute(database.connection());
        assert!(invalid_json.is_err());
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
