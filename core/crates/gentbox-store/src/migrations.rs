//! Schema migrations, tracked in SQLite's `user_version`.

use rusqlite::Connection;

use crate::error::StoreError;

/// Every migration, in order, as `(version, SQL)`. Versions start at 1 and
/// have no gaps. Add a migration by appending a file and a row; never edit one
/// that has shipped.
const MIGRATIONS: &[(u32, &str)] = &[(1, include_str!("migrations/0001_init.sql"))];

/// The newest schema version this build knows.
pub fn latest_version() -> u32 {
    MIGRATIONS.last().map_or(0, |(version, _)| *version)
}

pub(crate) fn current_version(connection: &Connection) -> Result<u32, StoreError> {
    let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    u32::try_from(version).map_err(|_| StoreError::corrupt("user_version", version))
}

/// Brings the file up to [`latest_version`]. Each migration runs in its own
/// transaction together with the version bump, so a crash leaves the file at
/// the last complete version.
pub(crate) fn migrate(connection: &mut Connection) -> Result<(), StoreError> {
    let found = current_version(connection)?;
    let supported = latest_version();
    if found > supported {
        return Err(StoreError::SchemaTooNew { found, supported });
    }
    for (version, sql) in MIGRATIONS.iter().filter(|(version, _)| *version > found) {
        let transaction = connection.transaction()?;
        transaction.execute_batch(sql)?;
        transaction.pragma_update(None, "user_version", version)?;
        transaction.commit()?;
    }
    Ok(())
}
