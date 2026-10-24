//! The hub lock (architecture §3), taken before the store is opened.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use gentbox_hub::{DATABASE_FILE_NAME, Hub, HubConfig, HubLock, LockError, NoDelegate, StartError};

#[tokio::test]
async fn a_second_hub_on_the_same_state_folder_fails_naming_the_holder() {
    let directory = tempfile::tempdir().unwrap();
    let first = Hub::start(HubConfig::new(directory.path()), Arc::new(NoDelegate), None).unwrap();
    let second = Hub::start(HubConfig::new(directory.path()), Arc::new(NoDelegate), None);
    match second {
        Err(StartError::Lock(LockError::Held { holder_pid })) => {
            assert_eq!(holder_pid, Some(std::process::id()));
        }
        other => panic!("expected the lock to be held, got {other:?}"),
    }
    let message = StartError::Lock(LockError::Held {
        holder_pid: Some(42),
    })
    .to_string();
    assert_eq!(message, "another hub is running (pid 42)");

    first.shutdown().await;
    let third = Hub::start(HubConfig::new(directory.path()), Arc::new(NoDelegate), None).unwrap();
    third.shutdown().await;
}

#[tokio::test]
async fn the_lock_is_taken_before_the_store_is_opened() {
    // Carried item 5: a hub that loses the lock never touches the database.
    let directory = tempfile::tempdir().unwrap();
    let held = HubLock::acquire(directory.path()).unwrap();
    let refused = Hub::start(HubConfig::new(directory.path()), Arc::new(NoDelegate), None);
    assert!(matches!(refused, Err(StartError::Lock(_))));
    assert!(!directory.path().join(DATABASE_FILE_NAME).exists());
    drop(held);
    let hub = Hub::start(HubConfig::new(directory.path()), Arc::new(NoDelegate), None).unwrap();
    assert!(directory.path().join(DATABASE_FILE_NAME).exists());
    hub.shutdown().await;
}

#[test]
fn starting_outside_a_runtime_is_an_error_not_a_panic() {
    let directory = tempfile::tempdir().unwrap();
    let outside = Hub::start(HubConfig::new(directory.path()), Arc::new(NoDelegate), None);
    assert!(matches!(outside, Err(StartError::NoRuntime)));
}
