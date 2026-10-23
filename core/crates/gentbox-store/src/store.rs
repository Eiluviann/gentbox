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
    /// Several processes may open the same file at once: the migration takes
    /// the write lock and the busy timeout (5 s) applies. All the same, the hub
    /// is the only writer by design, and `gentboxd` (PR 2b) takes the hub lock
    /// on the state folder before it opens the store.
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
        // Migrate first: it takes the write lock with `BEGIN IMMEDIATE`, so
        // concurrent first opens take turns instead of deadlocking on the
        // switch to WAL, which needs an exclusive lock.
        migrations::migrate(&mut connection)?;
        enable_wal(&connection)?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
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

/// How many times a switch to WAL that lost a lock race is tried again.
const WAL_ATTEMPTS: u32 = 1_000;

/// Switches the file to WAL. Two connections switching at the same moment can
/// deadlock, and SQLite then reports "busy" without waiting; whoever lost
/// finds the file already in WAL mode on the next look.
fn enable_wal(connection: &Connection) -> Result<(), StoreError> {
    for _ in 0..WAL_ATTEMPTS {
        match connection.query_row("PRAGMA journal_mode = WAL", [], |row| {
            row.get::<_, String>(0)
        }) {
            Ok(_) => return Ok(()),
            Err(rusqlite::Error::SqliteFailure(failure, _))
                if failure.code == rusqlite::ErrorCode::DatabaseBusy =>
            {
                std::thread::yield_now();
            }
            Err(error) => return Err(error.into()),
        }
    }
    connection
        .query_row("PRAGMA journal_mode = WAL", [], |row| {
            row.get::<_, String>(0)
        })
        .map(|_| ())
        .map_err(StoreError::from)
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

    #[test]
    fn a_default_thread_link_cannot_name_a_thread_of_another_topic() -> Result<(), StoreError> {
        let store = Store::open_in_memory()?;
        let connection = &store.connection;
        for topic in ["one", "two"] {
            connection.execute(
                "INSERT INTO topics (id, title, implicit, status, created_by_agent, created_at, \
                 updated_at) VALUES (?1, 'x', 0, 'open', 'a', 0, 0)",
                [topic],
            )?;
        }
        connection.execute(
            "INSERT INTO threads (id, topic_id, title, implicit, status, created_by_agent, \
             created_at, updated_at) VALUES ('t', 'one', 'x', 0, 'open', 'a', 0, 0)",
            [],
        )?;
        let link = |topic: &str| {
            connection.execute(
                "INSERT INTO conversation_default_threads \
                 (agent, conversation_id, topic_id, thread_id, last_used) \
                 VALUES ('a', 'c', ?1, 't', 0)",
                [topic],
            )
        };
        assert!(
            link("two").is_err(),
            "the link's topic must be the thread's"
        );
        assert!(link("one").is_ok());
        Ok(())
    }
}
