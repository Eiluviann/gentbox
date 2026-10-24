//! Migrations: from an empty file, on re-open, and the connection settings.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use gentbox_store::{Store, StoreError, latest_version};
use rusqlite::Connection;
use support::{NOW, file_store, plain, seed_group};

#[test]
fn an_empty_file_is_migrated_to_the_latest_version() {
    let (directory, store) = file_store();
    assert_eq!(store.schema_version().unwrap(), latest_version());
    assert_eq!(latest_version(), 2);
    assert!(directory.path().join("gentbox.db").exists());
}

#[test]
fn the_migrations_create_every_table() {
    let (directory, _store) = file_store();
    let connection = Connection::open(directory.path().join("gentbox.db")).unwrap();
    let mut statement = connection
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")
        .unwrap();
    let mut tables: Vec<String> = statement
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    tables.sort();
    assert_eq!(
        tables,
        [
            "actions",
            "conversation_default_threads",
            "conversations",
            "deliveries",
            "messages",
            "responses",
            "swept_messages",
            "threads",
            "topics",
        ]
    );
}

#[test]
fn messages_carry_thread_and_topic_ids() {
    let (directory, _store) = file_store();
    let connection = Connection::open(directory.path().join("gentbox.db")).unwrap();
    let mut statement = connection.prepare("PRAGMA table_info(messages)").unwrap();
    let columns: Vec<(String, bool)> = statement
        .query_map([], |row| Ok((row.get(1)?, row.get::<_, i64>(3)? == 1)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert!(columns.contains(&("thread_id".to_owned(), true)));
    assert!(columns.contains(&("topic_id".to_owned(), true)));
}

#[test]
fn reopening_an_existing_file_keeps_its_data_and_version() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("gentbox.db");
    {
        let store = Store::open(&path).unwrap();
        let repository = store.repository();
        seed_group(&repository, "one");
        repository
            .insert_message(&plain("first", "one", "one"), NOW)
            .unwrap();
    }
    let store = Store::open(&path).unwrap();
    assert_eq!(store.schema_version().unwrap(), latest_version());
    let message = store
        .repository()
        .message(&support::message_id("first"))
        .unwrap();
    assert_eq!(message.title, "Ingest tests pass");
}

#[test]
fn opening_a_file_twice_in_a_row_does_not_rerun_migrations() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("gentbox.db");
    for _ in 0..3 {
        let store = Store::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), latest_version());
    }
}

#[test]
fn a_file_from_a_newer_gentbox_is_refused() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("gentbox.db");
    drop(Store::open(&path).unwrap());
    Connection::open(&path)
        .unwrap()
        .pragma_update(None, "user_version", 99)
        .unwrap();
    let error = Store::open(&path).unwrap_err();
    assert!(matches!(
        error,
        StoreError::SchemaTooNew {
            found: 99,
            supported: 2
        }
    ));
}

#[test]
fn a_file_database_runs_in_wal_mode() {
    let (directory, _store) = file_store();
    let connection = Connection::open(directory.path().join("gentbox.db")).unwrap();
    let mode: String = connection
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
}

#[test]
fn an_in_memory_store_is_fully_migrated() {
    let store = Store::open_in_memory().unwrap();
    assert_eq!(store.schema_version().unwrap(), latest_version());
}

#[test]
fn a_version_1_file_migrates_to_version_2_and_keeps_its_data() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("gentbox.db");
    {
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(include_str!("../src/migrations/0001_init.sql"))
            .unwrap();
        connection.pragma_update(None, "user_version", 1).unwrap();
        connection
            .execute(
                "INSERT INTO topics (id, title, implicit, status, created_by_agent, created_at, \
                 updated_at) VALUES ('kept', 'Kept', 0, 'open', 'a', 0, 0)",
                [],
            )
            .unwrap();
    }
    let store = Store::open(&path).unwrap();
    assert_eq!(store.schema_version().unwrap(), 2);
    let topic = store
        .repository()
        .topic(&gentbox_core::ids::TopicId::new("kept"))
        .unwrap();
    assert_eq!(topic.title, "Kept");
    assert!(
        !store
            .repository()
            .was_swept(&gentbox_core::ids::MessageId::new("anything"))
            .unwrap()
    );
}

#[test]
fn swept_messages_remember_only_the_id_and_when() {
    let (directory, _store) = file_store();
    let connection = Connection::open(directory.path().join("gentbox.db")).unwrap();
    let mut statement = connection
        .prepare("PRAGMA table_info(swept_messages)")
        .unwrap();
    let columns: Vec<String> = statement
        .query_map([], |row| row.get(1))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(columns, ["id", "swept_at"]);
}
