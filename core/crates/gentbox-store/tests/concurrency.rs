//! A broken precondition must not poison the connection, and a second
//! connection waits for the write lock instead of failing at once.
// Contention between connections needs real threads; the store itself has none.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::disallowed_methods)]

mod support;

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use gentbox_core::resolve::Takeover;
use gentbox_store::Store;
use support::{NOW, explicit_default, message_id, plain, seed_group, thread_id, topic};

#[test]
fn a_panicking_precondition_leaves_the_store_usable_and_loses_nothing() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("gentbox.db");
    let mut store = Store::open(&path).unwrap();
    seed_group(&store.repository(), "one");
    seed_group(&store.repository(), "two");

    // A message whose topic is not its thread's topic.
    let skewed = plain("skewed", "two", "one");
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        let _ = store.repository().insert_message(&skewed, NOW);
    }));
    assert!(outcome.is_err());

    // A takeover of a thread that is not implicit.
    store.repository().insert_topic(&topic("three")).unwrap();
    store
        .repository()
        .insert_thread(&explicit_default("explicit", "three", "conversation-1"))
        .unwrap();
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        let _ = store.repository().insert_thread_with_takeover(
            &explicit_default("second", "three", "conversation-1"),
            &Takeover {
                replaces: thread_id("explicit"),
                inherits_mute: false,
            },
            NOW,
        );
    }));
    assert!(outcome.is_err());

    store
        .transaction(|repository| {
            repository.insert_message(&plain("in-transaction", "one", "one"), NOW)
        })
        .unwrap();
    store
        .repository()
        .insert_message(&plain("plain-write", "one", "one"), NOW)
        .unwrap();
    drop(store);

    let reopened = Store::open(&path).unwrap();
    for name in ["in-transaction", "plain-write"] {
        assert!(
            reopened
                .repository()
                .find_message(&message_id(name))
                .unwrap()
                .is_some(),
            "{name} was lost"
        );
    }
}

#[test]
fn two_stores_opening_a_new_file_at_once_both_succeed() {
    for _ in 0..40 {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("gentbox.db");
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let path = path.clone();
                thread::spawn(move || Store::open(&path).map(|_| ()))
            })
            .collect();
        for handle in handles {
            handle.join().unwrap().unwrap();
        }
    }
}

#[test]
fn a_second_connection_waits_for_a_write_in_progress() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("gentbox.db");
    let mut hub = Store::open(&path).unwrap();
    seed_group(&hub.repository(), "one");

    let (inside_transaction, wait_for_it) = mpsc::channel();
    let writer = thread::spawn(move || {
        hub.transaction(|repository| {
            repository.insert_message(&plain("first", "one", "one"), NOW)?;
            inside_transaction.send(()).unwrap();
            thread::sleep(Duration::from_millis(300));
            Ok(())
        })
        .unwrap();
    });
    wait_for_it.recv().unwrap();

    let other = Store::open(&path).unwrap();
    other
        .repository()
        .insert_message(&plain("second", "one", "one"), NOW)
        .unwrap();
    writer.join().unwrap();
    assert!(
        other
            .repository()
            .find_message(&message_id("first"))
            .unwrap()
            .is_some()
    );
}
