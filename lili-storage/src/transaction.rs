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
    use crate::models::AppStateRow;
    use crate::repository::{load_app_state, update_app_state};

    #[test]
    fn failed_short_transaction_rolls_back_all_database_work() {
        let root =
            std::env::temp_dir().join(format!("lili-storage-transaction-{}", uuid::Uuid::new_v4()));
        let paths = ApplicationPaths::from_root(root).unwrap();
        let mut database = database::open(&paths).unwrap();
        let result: QueryResult<()> = with_short_transaction(database.connection(), |connection| {
            update_app_state(
                connection,
                &AppStateRow {
                    id: 1,
                    selected_pet_id: Some("custom-pet".to_owned()),
                    window_placement_json: None,
                    reducer_revision: 1,
                    reducer_json: Some(r#"{"revision":1}"#.to_owned()),
                    spool_expired_drops: 0,
                    spool_limit_drops: 0,
                    spool_malformed_drops: 0,
                },
            )?;
            Err(diesel::result::Error::RollbackTransaction)
        });
        assert!(result.is_err());

        let persisted = load_app_state(database.connection()).unwrap();
        assert_eq!(persisted.selected_pet_id, None);
        assert_eq!(persisted.reducer_revision, 0);
        assert_eq!(persisted.reducer_json, None);
        drop(database);
        fs::remove_dir_all(paths.root()).unwrap();
    }
}
