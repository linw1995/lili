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
