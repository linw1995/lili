use diesel::Connection;
use diesel::result::QueryResult;
use diesel::sqlite::SqliteConnection;

/// Runs only database work in one short transaction.
pub fn with_short_transaction<T, F>(
    connection: &mut SqliteConnection,
    operation: F,
) -> QueryResult<T>
where
    F: FnOnce(&mut SqliteConnection) -> QueryResult<T>,
{
    connection.transaction(operation)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use diesel::prelude::*;

    use super::with_short_transaction;
    use crate::ApplicationPaths;
    use crate::database;
    use crate::models::NewSession;
    use crate::repository::insert_session;

    #[test]
    fn failed_short_transaction_rolls_back_all_database_work() {
        let root =
            std::env::temp_dir().join(format!("lili-storage-transaction-{}", uuid::Uuid::new_v4()));
        let paths = ApplicationPaths::from_root(root).unwrap();
        let mut database = database::open(&paths).unwrap();
        let result: QueryResult<()> = with_short_transaction(database.connection(), |connection| {
            insert_session(
                connection,
                &NewSession {
                    provider: "codex",
                    session_id: "session-1",
                    current_turn_id: None,
                    project_json: None,
                    updated_at_ms: 1,
                    last_event_id: "event-1",
                    ended: 0,
                },
            )?;
            Err(diesel::result::Error::RollbackTransaction)
        });
        assert!(result.is_err());

        let count = crate::schema::sessions::table
            .count()
            .get_result::<i64>(database.connection())
            .unwrap();
        assert_eq!(count, 0);
        drop(database);
        fs::remove_dir_all(paths.root()).unwrap();
    }
}
