//! [`GentboxHub`]: the hub, its local API and the commands the Mac UI sends.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use gentbox_core::classify::RuleSet;
use gentbox_core::ids::{MessageId, ThreadId, TopicId};
use gentbox_core::response::AnsweredVia;
use gentbox_core::topic::{CloseRequest, ThreadIdRef, ThreadRef, TopicIdRef, TopicRef};
use gentbox_hub::{
    ActionRequest, Hub, HubConfig, HubError, HubHandle, MessageThreadMove, ThreadMove,
};
use gentbox_ingest::{IngestServer, Limits};
use tokio::runtime::Runtime;

use crate::error::GentboxError;
use crate::events::{DelegateBridge, HubDelegate};
use crate::inbox::{Inbox, build_inbox};
use crate::records::PresenceUpdate;

/// Worker threads for the local API and the calls from Swift. The actor runs
/// on a thread of its own, so two are plenty.
const WORKER_THREADS: usize = 2;

/// Where the hub keeps its state.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct HubOptions {
    /// The state folder (architecture §3): the App Group container, or
    /// `~/Library/Application Support/Gentbox` for an ad hoc build. Created
    /// when missing.
    pub state_dir: String,
    /// The home folder, for `~` in `rules.toml`.
    pub home_dir: Option<String>,
}

/// The running hub and its listeners, taken on [`GentboxHub::stop`].
struct Running {
    hub: Hub,
    ingest: IngestServer,
}

/// The hub inside the Mac app: the actor, the store and the local API on the
/// Unix socket and loopback. Every query and command is `async` and runs on
/// the hub's own runtime; none blocks the caller's thread.
#[derive(uniffi::Object)]
pub struct GentboxHub {
    runtime: Runtime,
    handle: HubHandle,
    running: Mutex<Option<Running>>,
    port: u16,
    api_dir: String,
}

impl std::fmt::Debug for GentboxHub {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GentboxHub")
            .field("port", &self.port)
            .field("api_dir", &self.api_dir)
            .finish_non_exhaustive()
    }
}

#[uniffi::export]
impl GentboxHub {
    /// Starts the hub: takes the hub lock on the state folder, opens (and
    /// migrates) the store, reads `rules.toml`, and serves the local API.
    /// Blocks until the listeners are up, so call it off the main thread.
    ///
    /// `delegate` is called on the hub thread; see [`HubDelegate`].
    ///
    /// # Errors
    ///
    /// [`GentboxError::AnotherHubRunning`] when `gentboxd` or another build
    /// holds the state folder, [`GentboxError::CannotStart`] otherwise.
    #[uniffi::constructor]
    pub fn start(
        options: HubOptions,
        delegate: Arc<dyn HubDelegate>,
    ) -> Result<Arc<Self>, GentboxError> {
        let state_dir = PathBuf::from(&options.state_dir);
        std::fs::create_dir_all(&state_dir).map_err(|error| GentboxError::CannotStart {
            message: format!("cannot create {}: {error}", state_dir.display()),
        })?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(WORKER_THREADS)
            .thread_name("gentbox-ffi")
            .enable_all()
            .build()
            .map_err(|error| GentboxError::CannotStart {
                message: format!("cannot start the runtime: {error}"),
            })?;
        let mut config = HubConfig::new(&state_dir);
        config.rules = read_rules(&state_dir, options.home_dir.as_deref());
        // The app's UI answers: senders may wait (architecture §5.1).
        config.has_answer_channel = true;
        let hub = Hub::start(config, Arc::new(DelegateBridge(delegate)), None)?;
        let handle = hub.handle();
        let ingest = match runtime.block_on(IngestServer::start(
            handle.clone(),
            &state_dir,
            Limits::default(),
        )) {
            Ok(ingest) => ingest,
            Err(error) => {
                runtime.block_on(hub.shutdown());
                return Err(error.into());
            }
        };
        let port = ingest.port();
        tracing::info!(port, "gentbox hub running in the app");
        Ok(Arc::new(Self {
            runtime,
            handle,
            running: Mutex::new(Some(Running { hub, ingest })),
            port,
            api_dir: state_dir.join("API").display().to_string(),
        }))
    }

    /// Stops the listeners (removing `API/port` and the socket), then the
    /// hub after the commands already queued, and waits until the store is
    /// closed and the lock released. Later calls fail with `Stopped`. Blocks;
    /// call it once, at quit.
    pub fn stop(&self) {
        let running = self
            .running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(Running { hub, ingest }) = running {
            self.runtime.block_on(async move {
                ingest.shutdown().await;
                hub.shutdown().await;
            });
        }
    }

    /// The loopback port of the local API.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// The discovery folder: `hub.sock`, `token` and `port` (architecture §8).
    pub fn api_dir(&self) -> String {
        self.api_dir.clone()
    }

    /// The inbox, grouped topic → thread → messages, with the next-up card,
    /// the Closed section and the muted groups.
    ///
    /// # Errors
    ///
    /// [`GentboxError::Stopped`], [`GentboxError::Internal`].
    pub async fn inbox(&self) -> Result<Inbox, GentboxError> {
        let handle = self.handle.clone();
        let snapshot = self
            .run(async move { handle.inbox_snapshot().await })
            .await?;
        Ok(build_inbox(&snapshot))
    }

    /// A button press, answered on the Mac. The first answer wins.
    ///
    /// # Errors
    ///
    /// [`GentboxError::AlreadyClosed`] when the message closed first ("Already
    /// answered"), [`GentboxError::Refused`] for a button not offered now.
    pub async fn take_action(
        &self,
        message_id: String,
        action_id: String,
        value: Option<String>,
    ) -> Result<(), GentboxError> {
        let handle = self.handle.clone();
        let request = ActionRequest {
            action_id,
            value,
            via: AnsweredVia::Mac,
        };
        self.run(async move {
            handle
                .take_action(MessageId::new(message_id), request)
                .await
        })
        .await
        .map(drop)
    }

    /// Seen: no more re-pings, still answerable.
    ///
    /// # Errors
    ///
    /// [`GentboxError::AlreadyClosed`], [`GentboxError::NotFound`].
    pub async fn acknowledge(&self, message_id: String) -> Result<(), GentboxError> {
        let handle = self.handle.clone();
        self.run(async move { handle.acknowledge(MessageId::new(message_id)).await })
            .await
            .map(drop)
    }

    /// Remind me later (default 15 min); local, never sent to the agent.
    ///
    /// # Errors
    ///
    /// [`GentboxError::AlreadyClosed`], [`GentboxError::NotFound`].
    pub async fn snooze(
        &self,
        message_id: String,
        minutes: Option<u32>,
    ) -> Result<(), GentboxError> {
        let handle = self.handle.clone();
        self.run(async move { handle.snooze(MessageId::new(message_id), minutes).await })
            .await
            .map(drop)
    }

    /// Clears a message without answering it.
    ///
    /// # Errors
    ///
    /// [`GentboxError::Refused`] for `action_required` (use Done).
    pub async fn dismiss(&self, message_id: String) -> Result<(), GentboxError> {
        let handle = self.handle.clone();
        self.run(async move { handle.dismiss(MessageId::new(message_id)).await })
            .await
            .map(drop)
    }

    /// Mutes or unmutes a topic and so its threads: non-urgent messages go
    /// to the inbox only.
    ///
    /// # Errors
    ///
    /// [`GentboxError::NotFound`].
    pub async fn mute_topic(&self, topic_id: String, is_muted: bool) -> Result<(), GentboxError> {
        let handle = self.handle.clone();
        self.run(async move { handle.mute_topic(TopicId::new(topic_id), is_muted).await })
            .await
            .map(drop)
    }

    /// Mutes or unmutes a thread.
    ///
    /// # Errors
    ///
    /// [`GentboxError::NotFound`].
    pub async fn mute_thread(&self, thread_id: String, is_muted: bool) -> Result<(), GentboxError> {
        let handle = self.handle.clone();
        self.run(async move { handle.mute_thread(ThreadId::new(thread_id), is_muted).await })
            .await
            .map(drop)
    }

    /// Marks a topic's open messages read.
    ///
    /// # Errors
    ///
    /// [`GentboxError::NotFound`].
    pub async fn mark_topic_read(&self, topic_id: String) -> Result<(), GentboxError> {
        let handle = self.handle.clone();
        self.run(async move { handle.mark_topic_read(TopicId::new(topic_id)).await })
            .await
            .map(drop)
    }

    /// Marks a thread's open messages read.
    ///
    /// # Errors
    ///
    /// [`GentboxError::NotFound`].
    pub async fn mark_thread_read(&self, thread_id: String) -> Result<(), GentboxError> {
        let handle = self.handle.clone();
        self.run(async move { handle.mark_thread_read(ThreadId::new(thread_id)).await })
            .await
            .map(drop)
    }

    /// Closes a topic and its threads once nothing in them is open. Nothing
    /// on the Mac withdraws: "Withdrawn" is only the agent's own word; open
    /// items are answered, done or dismissed first.
    ///
    /// # Errors
    ///
    /// [`GentboxError::HasOpenMessages`] while anything in it is open,
    /// [`GentboxError::NotFound`], [`GentboxError::GeneralTopic`] for an agent's General topic.
    pub async fn close_topic(&self, topic_id: String) -> Result<(), GentboxError> {
        let handle = self.handle.clone();
        self.run(async move {
            handle
                .close_topic(TopicId::new(topic_id), CloseRequest::default(), None)
                .await
        })
        .await
        .map(drop)
    }

    /// Reopens a topic and the threads its closing closed.
    ///
    /// # Errors
    ///
    /// [`GentboxError::NotFound`].
    pub async fn reopen_topic(&self, topic_id: String) -> Result<(), GentboxError> {
        let handle = self.handle.clone();
        self.run(async move { handle.reopen_topic(TopicId::new(topic_id)).await })
            .await
            .map(drop)
    }

    /// Closes a thread once nothing in it is open.
    ///
    /// # Errors
    ///
    /// [`GentboxError::HasOpenMessages`] while anything in it is open,
    /// [`GentboxError::NotFound`].
    pub async fn close_thread(&self, thread_id: String) -> Result<(), GentboxError> {
        let handle = self.handle.clone();
        self.run(async move {
            handle
                .close_thread(ThreadId::new(thread_id), CloseRequest::default(), None)
                .await
        })
        .await
        .map(drop)
    }

    /// Reopens a thread and, when closed, its topic.
    ///
    /// # Errors
    ///
    /// [`GentboxError::NotFound`].
    pub async fn reopen_thread(&self, thread_id: String) -> Result<(), GentboxError> {
        let handle = self.handle.clone();
        self.run(async move { handle.reopen_thread(ThreadId::new(thread_id)).await })
            .await
            .map(drop)
    }

    /// Moves a thread, with its messages, to another topic.
    ///
    /// # Errors
    ///
    /// [`GentboxError::NotFound`], [`GentboxError::Refused`] when the target
    /// topic has a thread with the same key.
    pub async fn move_thread(
        &self,
        thread_id: String,
        topic_id: String,
    ) -> Result<(), GentboxError> {
        let handle = self.handle.clone();
        let request = ThreadMove {
            topic: TopicRef::Id(TopicIdRef {
                id: TopicId::new(topic_id),
            }),
        };
        self.run(async move {
            handle
                .move_thread_for_user(ThreadId::new(thread_id), request)
                .await
        })
        .await
        .map(drop)
    }

    /// Moves a message to another thread, or back to where the hub would put
    /// it (its conversation's default thread) with `thread_id: None`.
    ///
    /// # Errors
    ///
    /// [`GentboxError::NotFound`], [`GentboxError::Refused`].
    pub async fn move_message(
        &self,
        message_id: String,
        thread_id: Option<String>,
    ) -> Result<(), GentboxError> {
        let handle = self.handle.clone();
        let request = MessageThreadMove {
            thread: thread_id.map(|id| {
                ThreadRef::Id(ThreadIdRef {
                    id: ThreadId::new(id),
                })
            }),
            topic: None,
        };
        self.run(async move {
            handle
                .move_message_for_user(MessageId::new(message_id), request)
                .await
        })
        .await
        .map(drop)
    }

    /// Presence from the platform side: present, away or locked, and the
    /// frontmost app.
    ///
    /// # Errors
    ///
    /// [`GentboxError::Stopped`].
    pub async fn update_presence(&self, presence: PresenceUpdate) -> Result<(), GentboxError> {
        let handle = self.handle.clone();
        self.run(async move { handle.set_presence(presence.into()).await })
            .await
    }

    /// Focus on or off (from `INFocusStatusCenter`, PR 4).
    ///
    /// # Errors
    ///
    /// [`GentboxError::Stopped`].
    pub async fn update_focus(&self, is_focused: bool) -> Result<(), GentboxError> {
        let handle = self.handle.clone();
        self.run(async move { handle.set_focus(is_focused).await })
            .await
    }
}

impl GentboxHub {
    /// Runs a hub call on this crate's runtime and awaits it from whatever
    /// executor polls the caller (Swift's, through UniFFI): the join handle
    /// needs no runtime of its own.
    async fn run<T>(
        &self,
        call: impl Future<Output = Result<T, HubError>> + Send + 'static,
    ) -> Result<T, GentboxError>
    where
        T: Send + 'static,
    {
        self.runtime
            .spawn(call)
            .await
            .map_err(|_| GentboxError::Stopped)?
            .map_err(GentboxError::from)
    }
}

impl Drop for GentboxHub {
    /// A hub dropped without [`GentboxHub::stop`] still closes its store and
    /// removes its discovery files.
    fn drop(&mut self) {
        // `block_on` panics inside a runtime; the app drops the hub on its
        // main thread, so this only guards a misuse.
        if tokio::runtime::Handle::try_current().is_err() {
            self.stop();
        }
    }
}

/// `<state>/rules.toml`; missing is no rules, invalid is logged and ignored
/// until Settings (PR 5) shows the parse error in place.
fn read_rules(state_dir: &Path, home_dir: Option<&str>) -> RuleSet {
    let path = state_dir.join("rules.toml");
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) => {
            if error.kind() != std::io::ErrorKind::NotFound {
                tracing::error!(path = %path.display(), %error, "cannot read the rules");
            }
            return RuleSet::empty();
        }
    };
    let parsed = match home_dir {
        Some(home) => RuleSet::parse_with_home(&text, home),
        None => RuleSet::parse(&text),
    };
    parsed.unwrap_or_else(|error| {
        tracing::error!(path = %path.display(), line = ?error.line(), %error, "invalid rules; none applied");
        RuleSet::empty()
    })
}
