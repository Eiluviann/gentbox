//! Gentbox hub: the actor that serialises every state change (architecture
//! §4). Ingest handlers, the Mac UI (through FFI) and the timers send it
//! commands through a [`HubHandle`] and await the reply; long-polls wait on a
//! per-message `watch` channel.
//!
//! | Module | Role |
//! | --- | --- |
//! | [`commands`] | [`HubHandle`]: every command, its arguments and errors |
//! | `actor` | the actor: the store, the pipeline, answers, rescinds |
//! | [`waiters`] | live waiters and the long-poll |
//! | `timers` | re-ping, snooze, expiry, waiter lapse, the sweep |
//! | [`dispatch`] | [`HubEvent`]s to the UI delegate; the optional relay |
//! | [`lock`] | the hub lock, taken before the store is opened |
//! | [`clock`] | the clock, injectable for tests |
//!
//! The actor runs on a thread of its own with a single-threaded runtime, so
//! store IO never blocks the runtime that serves HTTP. v1 registers no phone
//! relay: `Hub::start` takes `None`.

mod actor;
pub mod clock;
pub mod commands;
mod config;
pub mod dispatch;
pub mod lock;
mod timers;
pub mod waiters;

use std::sync::Arc;

use gentbox_relay::Relay;
use gentbox_store::{Store, StoreError};
use thiserror::Error;
use tokio::sync::{mpsc, watch};

pub use clock::{Clock, ManualClock, SystemClock};
pub use commands::{
    ActionRequest, ConversationUpdate, HubError, HubHandle, InboxEntry, InboxSnapshot,
    IngestOutcome, IngestStatus, MessageGrouping, MessageThreadMove, Requester, ThreadMove,
    ThreadUpdate, TopicUpdate,
};
pub use config::{DATABASE_FILE_NAME, HubConfig};
pub use dispatch::{HubDelegate, HubEvent, NoDelegate};
pub use lock::{HubLock, LOCK_FILE_NAME, LockError};
pub use waiters::{LIVE_WAITER_WINDOW, MAX_WAIT, PendingWait, WaitStart};
// Store types that appear in the handle's signatures.
pub use gentbox_store::{MessageFilter, SweepReport, Upserted};

/// How many commands may queue before senders wait their turn.
const COMMAND_QUEUE: usize = 256;

/// Why a hub did not start.
#[derive(Debug, Error)]
pub enum StartError {
    /// Another hub holds the state folder, or the lock file is unusable.
    #[error(transparent)]
    Lock(#[from] LockError),
    /// The store could not be opened.
    #[error("cannot open the store: {0}")]
    Store(#[from] StoreError),
    /// The hub's thread or its runtime could not be started.
    #[error("cannot start the hub thread: {0}")]
    Thread(String),
}

/// A running hub: the actor on its own thread, and a handle to it.
#[derive(Debug)]
pub struct Hub {
    handle: HubHandle,
    stopped: watch::Receiver<bool>,
}

impl Hub {
    /// Starts a hub on a thread of its own: there it takes the hub lock on
    /// the state folder **before** opening the store, opens (and migrates)
    /// the store, restores timers for live messages, and runs the actor. No
    /// Tokio runtime is needed; the call blocks until the store is open, so
    /// from async code run it with `spawn_blocking`. `relay` is `None` in v1.
    ///
    /// # Errors
    ///
    /// [`StartError::Lock`] when another hub runs on the same state folder
    /// (the store is not touched), [`StartError::Store`],
    /// [`StartError::Thread`].
    pub fn start(
        config: HubConfig,
        delegate: Arc<dyn HubDelegate>,
        relay: Option<Arc<dyn Relay>>,
    ) -> Result<Self, StartError> {
        let (jobs, job_receiver) = mpsc::channel(COMMAND_QUEUE);
        let (signals, signal_receiver) = mpsc::unbounded_channel();
        let (stopped_sender, stopped) = watch::channel(false);
        let (ready, started) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("gentbox-hub".to_owned())
            .spawn(move || {
                let _announce = actor::StoppedOnDrop(stopped_sender);
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_time()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = ready.send(Err(StartError::Thread(error.to_string())));
                        return;
                    }
                };
                let state = match open_state(config, delegate, relay, signals) {
                    Ok(state) => state,
                    Err(error) => {
                        let _ = ready.send(Err(error));
                        return;
                    }
                };
                let _ = ready.send(Ok(()));
                runtime.block_on(actor::run(state, job_receiver, signal_receiver));
            })
            .map_err(|error| StartError::Thread(error.to_string()))?;
        started
            .recv()
            .map_err(|_| StartError::Thread("the hub thread ended while starting".to_owned()))??;
        Ok(Self {
            handle: HubHandle::new(jobs),
            stopped,
        })
    }

    /// A handle to the hub.
    pub fn handle(&self) -> HubHandle {
        self.handle.clone()
    }

    /// Turns true when the actor has ended (store closed, lock released),
    /// whether by [`Hub::shutdown`] or unexpectedly; a daemon watches it to
    /// exit instead of serving errors.
    pub fn stopped(&self) -> watch::Receiver<bool> {
        self.stopped.clone()
    }

    /// Stops the hub after the commands already queued, and waits until the
    /// store is closed and the lock released.
    pub async fn shutdown(self) {
        let _ = self.handle.shutdown().await;
        let mut stopped = self.stopped;
        let _ = stopped.wait_for(|is_stopped| *is_stopped).await;
    }
}

/// On the hub thread: the lock first, then the store, then the state.
fn open_state(
    config: HubConfig,
    delegate: Arc<dyn HubDelegate>,
    relay: Option<Arc<dyn Relay>>,
    signals: mpsc::UnboundedSender<waiters::WaiterSignal>,
) -> Result<actor::HubState, StartError> {
    let lock = HubLock::acquire(&config.state_dir)?;
    // Message bodies and answers live here: only the user may read them.
    let database = config.database_path();
    lock::prepare_private_file(&database).map_err(|source| LockError::Io {
        path: database.clone(),
        source,
    })?;
    let store = Store::open(&database)?;
    for suffix in ["-wal", "-shm"] {
        let companion = std::path::PathBuf::from(format!("{}{suffix}", database.display()));
        if companion.exists() {
            lock::make_private(&companion).map_err(|source| LockError::Io {
                path: companion.clone(),
                source,
            })?;
        }
    }
    Ok(actor::HubState::new(actor::StateParts {
        store,
        lock,
        clock: config.clock,
        rules: config.rules,
        policy: config.policy,
        sources: config.sources,
        retention: config.retention,
        has_answer_channel: config.has_answer_channel,
        dispatcher: dispatch::Dispatcher::new(delegate, relay),
        signals,
    })?)
}
