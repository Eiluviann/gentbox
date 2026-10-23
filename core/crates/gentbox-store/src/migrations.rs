//! Schema migrations, tracked in SQLite's `user_version`.

use rusqlite::{Connection, TransactionBehavior};

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

/// Brings the file up to [`latest_version`], in one transaction together with
/// the version bump, so a crash leaves the file at the last complete version.
///
/// The transaction starts with `BEGIN IMMEDIATE` and the version is read again
/// inside it: two processes opening a new file at once take turns, and the
/// second finds the schema already there.
pub(crate) fn migrate(connection: &mut Connection) -> Result<(), StoreError> {
    let supported = latest_version();
    let found = current_version(connection)?;
    if found > supported {
        return Err(StoreError::SchemaTooNew { found, supported });
    }
    if found == supported {
        return Ok(());
    }
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let found = current_version(&transaction)?;
    if found > supported {
        return Err(StoreError::SchemaTooNew { found, supported });
    }
    for (version, sql) in MIGRATIONS.iter().filter(|(version, _)| *version > found) {
        transaction.execute_batch(sql)?;
        transaction.pragma_update(None, "user_version", version)?;
    }
    transaction.commit()?;
    Ok(())
}
