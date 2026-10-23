//! Opening the database file and grouping writes into transactions.

use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, TransactionBehavior};

use crate::error::StoreError;
use crate::migrations;
use crate::repository::Repository;

/// How long SQLite waits for a lock another connection holds.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// The SQLite database behind the hub. Synchronous: the hub actor owns one and
/// is its only writer.
#[derive(Debug)]
pub struct Store {
    connection: Connection,
}

impl Store {
    /// Opens (creating when missing) the database at `path` in WAL mode with
    /// foreign keys enforced, and migrates it to the newest schema.
    ///
    /// # Errors
    ///
    /// [`StoreError::SchemaTooNew`] when the file was written by a newer
    /// Gentbox; [`StoreError::Database`] when SQLite cannot open it.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let connection = Connection::open(path)?;
        Self::prepare(connection)
    }

    /// An in-memory database, for tests and tools. It is not WAL (there is no
    /// file) but is otherwise identical.
    pub fn open_in_memory() -> Result<Self, StoreError> {
        Self::prepare(Connection::open_in_memory()?)
    }

    fn prepare(mut connection: Connection) -> Result<Self, StoreError> {
        connection.busy_timeout(BUSY_TIMEOUT)?;
        connection.pragma_update(None, "foreign_keys", true)?;
        connection.query_row("PRAGMA journal_mode = WAL", [], |row| {
            row.get::<_, String>(0)
        })?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        migrations::migrate(&mut connection)?;
        Ok(Self { connection })
    }

    /// The schema version of the open file.
    pub fn schema_version(&self) -> Result<u32, StoreError> {
        migrations::current_version(&self.connection)
    }

    /// The repository over the connection. Every method that writes more than
    /// one row is atomic by itself; use [`Store::transaction`] to make several
    /// calls atomic together.
    pub fn repository(&self) -> Repository<'_> {
        Repository::new(&self.connection)
    }

    /// Runs `work` in one write transaction: committed when it returns `Ok`,
    /// rolled back when it returns `Err`.
    pub fn transaction<T>(
        &mut self,
        work: impl FnOnce(&Repository<'_>) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let value = work(&Repository::new(&transaction))?;
        transaction.commit()?;
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn foreign_keys_are_enforced() -> Result<(), StoreError> {
        let store = Store::open_in_memory()?;
        let enforced: bool = store
            .connection
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))?;
        assert!(enforced);
        let orphan = store.connection.execute(
            "INSERT INTO threads (id, topic_id, title, implicit, status, created_by_agent, \
             created_at, updated_at) VALUES ('t', 'no-such-topic', 'x', 0, 'open', 'a', 0, 0)",
            [],
        );
        assert!(orphan.is_err());
        Ok(())
    }
}
