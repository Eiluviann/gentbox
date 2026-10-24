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
//! v1 registers no phone relay: `Hub::start` takes `None`.

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
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

pub use clock::{Clock, ManualClock, SystemClock};
pub use commands::{
    ActionRequest, ConversationUpdate, HubError, HubHandle, IngestOutcome, IngestStatus,
    MessageGrouping, MessageThreadMove, Requester, ThreadMove, ThreadUpdate, TopicUpdate,
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
    /// `Hub::start` was called outside a Tokio runtime.
    #[error("the hub needs a Tokio runtime")]
    NoRuntime,
}

/// A running hub: the actor task and a handle to it.
#[derive(Debug)]
pub struct Hub {
    handle: HubHandle,
    task: JoinHandle<()>,
}

impl Hub {
    /// Starts a hub on the current Tokio runtime: takes the hub lock on the
    /// state folder **before** opening the store, opens (and migrates) the
    /// store, restores timers for live messages, sweeps, and spawns the
    /// actor. `relay` is `None` in v1.
    ///
    /// # Errors
    ///
    /// [`StartError::Lock`] when another hub runs on the same state folder
    /// (the store is not touched), [`StartError::Store`],
    /// [`StartError::NoRuntime`].
    pub fn start(
        config: HubConfig,
        delegate: Arc<dyn HubDelegate>,
        relay: Option<Arc<dyn Relay>>,
    ) -> Result<Self, StartError> {
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| StartError::NoRuntime)?;
        let lock = HubLock::acquire(&config.state_dir)?;
        let store = Store::open(config.database_path())?;
        let (jobs, job_receiver) = mpsc::channel(COMMAND_QUEUE);
        let (signals, signal_receiver) = mpsc::unbounded_channel();
        let state = actor::HubState::new(actor::StateParts {
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
        })?;
        let task = runtime.spawn(actor::run(state, job_receiver, signal_receiver));
        Ok(Self {
            handle: HubHandle::new(jobs),
            task,
        })
    }

    /// A handle to the hub.
    pub fn handle(&self) -> HubHandle {
        self.handle.clone()
    }

    /// Stops the hub after the commands already queued, and waits until the
    /// store is closed and the lock released.
    pub async fn shutdown(self) {
        let _ = self.handle.shutdown().await;
        let _ = self.task.await;
    }
}
