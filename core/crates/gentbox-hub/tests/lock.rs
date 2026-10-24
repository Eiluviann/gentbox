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
fn a_hub_starts_without_a_tokio_runtime_and_runs_its_store_on_its_own_thread() {
    let directory = tempfile::tempdir().unwrap();
    let hub = Hub::start(HubConfig::new(directory.path()), Arc::new(NoDelegate), None).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        hub.handle().sweep().await.unwrap();
        hub.shutdown().await;
    });
    // The lock is free once shutdown returns.
    HubLock::acquire(directory.path()).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn the_state_folder_and_the_database_are_private() {
    use std::os::unix::fs::PermissionsExt;
    let mode =
        |path: &std::path::Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    let parent = tempfile::tempdir().unwrap();
    let state = parent.path().join("state");
    let hub = Hub::start(HubConfig::new(&state), Arc::new(NoDelegate), None).unwrap();
    hub.handle().sweep().await.unwrap();
    assert_eq!(mode(&state), 0o700);
    for file in ["hub.lock", DATABASE_FILE_NAME, "hub.db-wal", "hub.db-shm"] {
        let path = state.join(file);
        if path.exists() {
            assert_eq!(mode(&path), 0o600, "{file}");
        }
    }
    assert!(state.join(DATABASE_FILE_NAME).exists());
    hub.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn a_database_left_readable_by_others_is_tightened() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join(DATABASE_FILE_NAME);
    std::fs::write(&database, b"").unwrap();
    std::fs::set_permissions(&database, std::fs::Permissions::from_mode(0o644)).unwrap();
    let hub = Hub::start(HubConfig::new(directory.path()), Arc::new(NoDelegate), None).unwrap();
    assert_eq!(
        std::fs::metadata(&database).unwrap().permissions().mode() & 0o777,
        0o600
    );
    hub.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn commands_caught_by_a_shutdown_answer_stopped_not_internal() {
    let directory = tempfile::tempdir().unwrap();
    let hub = Hub::start(HubConfig::new(directory.path()), Arc::new(NoDelegate), None).unwrap();
    let handle = hub.handle();
    let calls: Vec<_> = (0..64)
        .map(|_| {
            let handle = handle.clone();
            tokio::spawn(async move { handle.sweep().await })
        })
        .collect();
    let stopped = hub.stopped();
    hub.shutdown().await;
    assert!(*stopped.borrow(), "the hub reports it stopped");
    for call in calls {
        match call.await.unwrap() {
            Ok(_) | Err(gentbox_hub::HubError::Stopped) => {}
            Err(other) => panic!("expected Stopped, got {other:?}"),
        }
    }
    assert!(matches!(
        handle.sweep().await,
        Err(gentbox_hub::HubError::Stopped)
    ));
}
