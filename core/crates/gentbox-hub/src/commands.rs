//! The hub's commands: [`HubHandle`] and the types its methods take and
//! return.
//!
//! Every method sends one job to the actor over an `mpsc` channel and awaits
//! its reply, so commands never interleave: "first answer wins" and the
//! supersede, rescind and Defer races are impossible by construction
//! (architecture §4). Ingest handlers use the sender methods; the Mac UI
//! (through FFI) uses the rest. There is no answer method for senders.

use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration as StdDuration;

use gentbox_core::classify::RuleSet;
use gentbox_core::ids::{ConversationKey, MessageId, ThreadId, TopicId};
use gentbox_core::lifecycle::{ActionError, ActionOutcome, LifecycleState, ResolutionReason};
use gentbox_core::message::{Action, Conversation, Message, MessageInput};
use gentbox_core::policy::{Policy, Presence};
use gentbox_core::prepare::{IngestRejection, IngestSettings};
use gentbox_core::rescind::{RescindReason, RescindRequest};
use gentbox_core::resolve::GroupingError;
use gentbox_core::response::{ActionResponse, AnsweredVia, IngestResponse, IngestWarning};
use gentbox_core::topic::{
    CloseRequest, GroupStatus, ThreadCreate, ThreadRecord, ThreadRef, TopicCreate, TopicRecord,
    TopicRef, TopicStyle,
};
use gentbox_store::{Conflict, EntityKind, MessageFilter, StoreError, SweepReport, Upserted};
use serde::{Deserialize, Deserializer};
use thiserror::Error;
use tokio::sync::{mpsc, oneshot};

use crate::actor::HubState;
use crate::waiters::WaitStart;

/// Why a command failed. Not-found and refusal outcomes are expected results
/// of valid use; [`HubError::Store`], [`HubError::Stopped`] and
/// [`HubError::Panicked`] are the hub's own failures.
#[derive(Debug, Error)]
pub enum HubError {
    /// No message has this id (and retention did not delete one with it).
    #[error("message {0} not found")]
    MessageNotFound(MessageId),
    /// No topic has this id.
    #[error("topic {0} not found")]
    TopicNotFound(TopicId),
    /// No thread has this id.
    #[error("thread {0} not found")]
    ThreadNotFound(ThreadId),
    /// The message belongs to another agent or conversation.
    #[error("the message belongs to another agent or conversation")]
    NotOwner,
    /// The message is closed: a late answer ("Already answered"), a late
    /// rescind, or a change to a withdrawn message.
    #[error("the message is already {state:?}")]
    MessageClosed {
        /// Its terminal state.
        state: LifecycleState,
        /// Why it closed.
        resolution: Option<ResolutionReason>,
    },
    /// The action id is not one of the message's buttons.
    #[error("the message has no action with this id")]
    UnknownAction,
    /// The button exists but is not offered now (an answer without a live
    /// waiter, Defer after Defer).
    #[error("the action is not offered now")]
    ActionNotOffered,
    /// The answer's value is missing or invalid (a reply needs text).
    #[error("invalid answer value: {0}")]
    InvalidAnswerValue(&'static str),
    /// An `action_required` message is closed with Done, never cleared.
    #[error("an action_required message is closed with Done")]
    UseDoneInstead,
    /// Defer on a message that is not `deferrable`.
    #[error("only a deferrable message can be deferred")]
    NotDeferrable,
    /// A step index the message does not have.
    #[error("step {0} does not exist")]
    UnknownStep(usize),
    /// A rescind by correlation key named no key.
    #[error("a rescind by correlation key needs at least one correlation key")]
    MissingCorrelationKeys,
    /// Agents may not edit an implicit topic or thread (409).
    #[error("implicit topics and threads are managed by the hub")]
    ImplicitGroup,
    /// A key or default-thread uniqueness rule (409).
    #[error(transparent)]
    Conflict(Conflict),
    /// The message failed the pipeline (422 with a code).
    #[error(transparent)]
    Rejected(IngestRejection),
    /// A topic or thread could not be resolved or created (422 with a code).
    #[error(transparent)]
    Grouping(GroupingError),
    /// The store failed.
    #[error("store error: {0}")]
    Store(StoreError),
    /// The hub has stopped.
    #[error("the hub has stopped")]
    Stopped,
    /// The command panicked; the hub keeps running and the store rolled back.
    #[error("the command failed inside the hub")]
    Panicked,
}

impl From<StoreError> for HubError {
    fn from(error: StoreError) -> Self {
        match error {
            StoreError::NotFound {
                entity: EntityKind::Message,
                id,
            } => Self::MessageNotFound(MessageId::new(id)),
            StoreError::NotFound {
                entity: EntityKind::Topic,
                id,
            } => Self::TopicNotFound(TopicId::new(id)),
            StoreError::NotFound {
                entity: EntityKind::Thread,
                id,
            } => Self::ThreadNotFound(ThreadId::new(id)),
            StoreError::Conflict(conflict) => Self::Conflict(conflict),
            other => Self::Store(other),
        }
    }
}

impl From<ActionError> for HubError {
    fn from(error: ActionError) -> Self {
        match error {
            ActionError::Closed { state, resolution } => Self::MessageClosed { state, resolution },
            ActionError::NotDeferrable => Self::NotDeferrable,
            ActionError::UseDoneInstead => Self::UseDoneInstead,
            ActionError::UnknownAction => Self::UnknownAction,
            ActionError::SnoozeOutOfRange => Self::InvalidAnswerValue("snooze out of range"),
        }
    }
}

/// Who asks for a change: the agent, and the conversation when it says.
/// An agent withdraws and moves only its own messages (ADR 0007).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requester {
    /// `source.agent`.
    pub agent: String,
    /// `conversation.id`, when the request names one.
    pub conversation_id: Option<String>,
}

impl Requester {
    /// An agent, without a conversation.
    pub fn agent(agent: impl Into<String>) -> Self {
        Self {
            agent: agent.into(),
            conversation_id: None,
        }
    }

    /// Whether the message is this requester's.
    pub fn owns(&self, message: &Message) -> bool {
        message.source.agent == self.agent
            && self
                .conversation_id
                .as_ref()
                .is_none_or(|conversation| *conversation == message.conversation.id)
    }
}

/// 201 or 200.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngestStatus {
    /// A new message was stored (201).
    Created,
    /// Nothing new was stored: an idempotent retry, a repeat absorbed by an
    /// open message (fold, dedupe); the response names that message (200).
    Existing,
}

/// The result of `POST /v1/messages`.
#[derive(Debug, Clone, PartialEq)]
pub struct IngestOutcome {
    /// 201 or 200.
    pub status: IngestStatus,
    /// The body.
    pub response: IngestResponse,
}

/// A button press from the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionRequest {
    /// The pressed button's id; it must be one of the message's buttons.
    pub action_id: String,
    /// The reply text (ignored for other kinds; a choice returns its own
    /// value).
    pub value: Option<String>,
    /// The channel it came through.
    pub via: AnsweredVia,
}

/// The result of `PUT /v1/conversations/{agent}/{id}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationUpdate {
    /// The stored conversation.
    pub conversation: Conversation,
    /// What the hub changed (an ignored title, a redacted secret).
    pub warnings: Vec<IngestWarning>,
}

/// `PATCH /v1/topics/{id}` (`$defs/topicUpdate`). Only the given fields
/// change; `summary: null` clears the summary.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TopicUpdate {
    /// A new title (through the guard and the secret scan).
    #[serde(default)]
    pub title: Option<String>,
    /// `Some(None)` clears the summary.
    #[serde(default, deserialize_with = "present_or_null")]
    pub summary: Option<Option<String>>,
    /// New style hints.
    #[serde(default)]
    pub style: Option<TopicStyle>,
    /// A new kind hint.
    #[serde(default)]
    pub kind: Option<String>,
}

/// `PATCH /v1/threads/{id}` (`$defs/threadUpdate`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThreadUpdate {
    /// A new title (through the guard and the secret scan).
    #[serde(default)]
    pub title: Option<String>,
    /// `Some(None)` clears the summary.
    #[serde(default, deserialize_with = "present_or_null")]
    pub summary: Option<Option<String>>,
    /// A new kind hint.
    #[serde(default)]
    pub kind: Option<String>,
}

/// `PUT /v1/threads/{id}/topic` (`$defs/threadMove`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThreadMove {
    /// The target topic; a key creates it on first use.
    pub topic: TopicRef,
}

/// `PUT /v1/messages/{id}/thread` (`$defs/messageThreadMove`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageThreadMove {
    /// The target thread, or `None` for the default resolution.
    pub thread: Option<ThreadRef>,
    /// The topic, when the thread is named by key or re-resolved.
    #[serde(default)]
    pub topic: Option<TopicRef>,
}

/// Where a moved message is now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageGrouping {
    /// The message.
    pub message_id: MessageId,
    /// Its topic.
    pub topic_id: TopicId,
    /// Its thread.
    pub thread_id: ThreadId,
}

/// A JSON field that may be absent (`None`), `null` (`Some(None)`) or set.
fn present_or_null<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// One command, run on the actor.
pub(crate) type Job = Box<dyn FnOnce(&mut HubState) + Send>;

/// The way into a running hub. Cheap to clone; every clone talks to the same
/// actor.
#[derive(Clone)]
pub struct HubHandle {
    jobs: mpsc::Sender<Job>,
}

impl fmt::Debug for HubHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HubHandle")
            .field("is_closed", &self.jobs.is_closed())
            .finish()
    }
}

impl HubHandle {
    pub(crate) fn new(jobs: mpsc::Sender<Job>) -> Self {
        Self { jobs }
    }

    /// Runs `work` on the actor and returns its result.
    async fn call<T>(
        &self,
        work: impl FnOnce(&mut HubState) -> Result<T, HubError> + Send + 'static,
    ) -> Result<T, HubError>
    where
        T: Send + 'static,
    {
        let (reply, answer) = oneshot::channel();
        let job: Job = Box::new(move |state| {
            // The caller may have given up (a client hung up); the work is
            // done either way.
            let _ = reply.send(work(state));
        });
        self.jobs.send(job).await.map_err(|_| HubError::Stopped)?;
        // A dropped reply means the hub stopped with the job still queued, or
        // the job panicked and the actor caught it.
        answer.await.map_err(|_| {
            if self.jobs.is_closed() {
                HubError::Stopped
            } else {
                HubError::Panicked
            }
        })?
    }

    // -----------------------------------------------------------------
    // Senders (the sender token): post, wait, rescind, progress, groups
    // -----------------------------------------------------------------

    /// `POST /v1/messages`: the pipeline of architecture §5.1 after schema
    /// validation.
    ///
    /// # Errors
    ///
    /// [`HubError::Rejected`] and [`HubError::Grouping`] for a message the
    /// hub refuses (422).
    pub async fn ingest(&self, input: MessageInput) -> Result<IngestOutcome, HubError> {
        self.call(move |hub| hub.ingest(input)).await
    }

    /// Starts a long-poll: ready at once when the outcome is known, else a
    /// [`WaitStart::Waiting`] that counts as a live waiter until it ends.
    ///
    /// # Errors
    ///
    /// [`HubError::MessageNotFound`] for an id the hub never issued (a swept
    /// one answers `gone`).
    pub async fn begin_wait(&self, id: MessageId) -> Result<WaitStart, HubError> {
        self.call(move |hub| hub.begin_wait(&id)).await
    }

    /// `GET /v1/messages/{id}/response?wait=…`: waits up to `wait` (at most
    /// 55 s) for the answer.
    ///
    /// # Errors
    ///
    /// As [`HubHandle::begin_wait`].
    pub async fn wait_for_response(
        &self,
        id: MessageId,
        wait: StdDuration,
    ) -> Result<ActionResponse, HubError> {
        Ok(self.begin_wait(id).await?.wait(wait).await)
    }

    /// `DELETE /v1/messages/{id}`: withdraws one of the requester's messages.
    ///
    /// # Errors
    ///
    /// [`HubError::NotOwner`] for another agent's message,
    /// [`HubError::MessageClosed`] when it is already closed.
    pub async fn rescind(
        &self,
        id: MessageId,
        reason: RescindReason,
        requester: Requester,
    ) -> Result<ActionResponse, HubError> {
        self.call(move |hub| hub.rescind(&id, reason, &requester))
            .await
    }

    /// `POST /v1/conversations/{agent}/{id}/rescind`: withdraws the
    /// conversation's live messages with the listed correlation keys.
    ///
    /// # Errors
    ///
    /// [`HubError::MissingCorrelationKeys`] when no key is given.
    pub async fn rescind_by_correlation_keys(
        &self,
        conversation: ConversationKey,
        request: RescindRequest,
    ) -> Result<Vec<MessageId>, HubError> {
        self.call(move |hub| hub.rescind_by_correlation_keys(&conversation, &request))
            .await
    }

    /// `POST /v1/conversations/{agent}/{id}/progress` (architecture §7.6).
    ///
    /// # Errors
    ///
    /// Only the hub's own failures.
    pub async fn progress(
        &self,
        conversation: ConversationKey,
    ) -> Result<Vec<MessageId>, HubError> {
        self.call(move |hub| hub.progress(&conversation)).await
    }

    /// `PUT /v1/conversations/{agent}/{id}`: registers or renames a
    /// conversation; its implicit threads follow the title.
    ///
    /// # Errors
    ///
    /// Only the hub's own failures.
    pub async fn upsert_conversation(
        &self,
        key: ConversationKey,
        conversation: Conversation,
    ) -> Result<ConversationUpdate, HubError> {
        self.call(move |hub| hub.upsert_conversation(&key, conversation))
            .await
    }

    /// `POST /v1/topics`: an upsert by key.
    ///
    /// # Errors
    ///
    /// [`HubError::Grouping`] with `title_looks_like_command` (422).
    pub async fn create_topic(
        &self,
        agent: String,
        request: TopicCreate,
    ) -> Result<Upserted<TopicRecord>, HubError> {
        self.call(move |hub| hub.create_topic(&agent, &request))
            .await
    }

    /// `GET /v1/topics?status=…`.
    ///
    /// # Errors
    ///
    /// Only the hub's own failures.
    pub async fn list_topics(
        &self,
        status: Option<GroupStatus>,
    ) -> Result<Vec<TopicRecord>, HubError> {
        self.call(move |hub| Ok(hub.store().list_topic_records(status)?))
            .await
    }

    /// `GET /v1/topics/{id}`.
    ///
    /// # Errors
    ///
    /// [`HubError::TopicNotFound`].
    pub async fn topic(&self, id: TopicId) -> Result<TopicRecord, HubError> {
        self.call(move |hub| Ok(hub.store().topic_record(&id)?))
            .await
    }

    /// `PATCH /v1/topics/{id}`.
    ///
    /// # Errors
    ///
    /// [`HubError::ImplicitGroup`] for a General topic (409),
    /// [`HubError::Grouping`] for a flagged title (422).
    pub async fn update_topic(
        &self,
        id: TopicId,
        update: TopicUpdate,
    ) -> Result<TopicRecord, HubError> {
        self.call(move |hub| hub.update_topic(&id, update)).await
    }

    /// `POST /v1/topics/{id}/close`: closes the topic and its open threads;
    /// with `rescindOpenMessages`, withdraws the open messages in it (only
    /// the requester's, when there is one; all of them for the UI).
    ///
    /// # Errors
    ///
    /// [`HubError::TopicNotFound`].
    pub async fn close_topic(
        &self,
        id: TopicId,
        request: CloseRequest,
        requester: Option<Requester>,
    ) -> Result<TopicRecord, HubError> {
        self.call(move |hub| hub.close_topic(&id, &request, requester.as_ref()))
            .await
    }

    /// `POST /v1/topics/{id}/reopen`: reopens it and the threads its closing
    /// closed.
    ///
    /// # Errors
    ///
    /// [`HubError::TopicNotFound`].
    pub async fn reopen_topic(&self, id: TopicId) -> Result<TopicRecord, HubError> {
        self.call(move |hub| hub.reopen_topic(&id)).await
    }

    /// `POST /v1/topics/{id}/rescind`: withdraws the open messages in the
    /// topic (only the requester's, when there is one) without closing it.
    ///
    /// # Errors
    ///
    /// [`HubError::TopicNotFound`].
    pub async fn rescind_topic(
        &self,
        id: TopicId,
        reason: RescindReason,
        requester: Option<Requester>,
    ) -> Result<Vec<MessageId>, HubError> {
        self.call(move |hub| hub.rescind_in_topic(&id, reason, requester.as_ref()))
            .await
    }

    /// `POST /v1/threads`: an upsert by key within the topic, with the
    /// takeover of ADR 0009.
    ///
    /// # Errors
    ///
    /// [`HubError::Grouping`] for a flagged title or an unknown topic (422).
    pub async fn create_thread(
        &self,
        agent: String,
        request: ThreadCreate,
    ) -> Result<Upserted<ThreadRecord>, HubError> {
        self.call(move |hub| hub.create_thread(&agent, &request))
            .await
    }

    /// `GET /v1/threads?topic=…&status=…`.
    ///
    /// # Errors
    ///
    /// Only the hub's own failures.
    pub async fn list_threads(
        &self,
        topic: Option<TopicId>,
        status: Option<GroupStatus>,
    ) -> Result<Vec<ThreadRecord>, HubError> {
        self.call(move |hub| Ok(hub.store().list_thread_records(topic.as_ref(), status)?))
            .await
    }

    /// `GET /v1/threads/{id}`.
    ///
    /// # Errors
    ///
    /// [`HubError::ThreadNotFound`].
    pub async fn thread(&self, id: ThreadId) -> Result<ThreadRecord, HubError> {
        self.call(move |hub| Ok(hub.store().thread_record(&id)?))
            .await
    }

    /// `PATCH /v1/threads/{id}`.
    ///
    /// # Errors
    ///
    /// [`HubError::ImplicitGroup`] (409), [`HubError::Grouping`] (422).
    pub async fn update_thread(
        &self,
        id: ThreadId,
        update: ThreadUpdate,
    ) -> Result<ThreadRecord, HubError> {
        self.call(move |hub| hub.update_thread(&id, update)).await
    }

    /// `POST /v1/threads/{id}/close`.
    ///
    /// # Errors
    ///
    /// [`HubError::ThreadNotFound`].
    pub async fn close_thread(
        &self,
        id: ThreadId,
        request: CloseRequest,
        requester: Option<Requester>,
    ) -> Result<ThreadRecord, HubError> {
        self.call(move |hub| hub.close_thread(&id, &request, requester.as_ref()))
            .await
    }

    /// `POST /v1/threads/{id}/reopen`: reopens it and, when closed, its topic.
    ///
    /// # Errors
    ///
    /// [`HubError::ThreadNotFound`].
    pub async fn reopen_thread(&self, id: ThreadId) -> Result<ThreadRecord, HubError> {
        self.call(move |hub| hub.reopen_thread(&id)).await
    }

    /// `POST /v1/threads/{id}/rescind`.
    ///
    /// # Errors
    ///
    /// [`HubError::ThreadNotFound`].
    pub async fn rescind_thread(
        &self,
        id: ThreadId,
        reason: RescindReason,
        requester: Option<Requester>,
    ) -> Result<Vec<MessageId>, HubError> {
        self.call(move |hub| hub.rescind_in_thread(&id, reason, requester.as_ref()))
            .await
    }

    /// `PUT /v1/threads/{id}/topic`: moves a thread, with its messages.
    ///
    /// # Errors
    ///
    /// [`HubError::Conflict`] when the key exists in the target (409),
    /// [`HubError::Grouping`] for an unknown target topic (422).
    pub async fn move_thread(
        &self,
        id: ThreadId,
        request: ThreadMove,
        requester: Requester,
    ) -> Result<ThreadRecord, HubError> {
        self.call(move |hub| hub.move_thread(&id, &request, &requester))
            .await
    }

    /// `PUT /v1/messages/{id}/thread`: moves one of the requester's messages.
    ///
    /// # Errors
    ///
    /// [`HubError::NotOwner`], [`HubError::Grouping`].
    pub async fn move_message(
        &self,
        id: MessageId,
        request: MessageThreadMove,
        requester: Requester,
    ) -> Result<MessageGrouping, HubError> {
        self.call(move |hub| hub.move_message(&id, &request, &requester))
            .await
    }

    /// `POST /v1/presence/focus`, and Focus from `INFocusStatusCenter`.
    ///
    /// # Errors
    ///
    /// Only [`HubError::Stopped`].
    pub async fn set_focus(&self, is_focused: bool) -> Result<(), HubError> {
        self.call(move |hub| {
            hub.set_focus(is_focused);
            Ok(())
        })
        .await
    }

    // -----------------------------------------------------------------
    // The UI (in-process, through FFI): answers and the inbox
    // -----------------------------------------------------------------

    /// A button press: the first answer wins; a later one gets
    /// [`HubError::MessageClosed`] ("Already answered").
    ///
    /// # Errors
    ///
    /// [`HubError::UnknownAction`] for an id that is not one of the
    /// message's buttons, [`HubError::ActionNotOffered`] for one not offered
    /// now, [`HubError::MessageClosed`] for a closed message.
    pub async fn take_action(
        &self,
        id: MessageId,
        request: ActionRequest,
    ) -> Result<ActionOutcome, HubError> {
        self.call(move |hub| hub.take_action(&id, &request)).await
    }

    /// Seen: the popup closed, the inbox row opened.
    ///
    /// # Errors
    ///
    /// [`HubError::MessageClosed`] for a closed message.
    pub async fn acknowledge(&self, id: MessageId) -> Result<Message, HubError> {
        self.call(move |hub| hub.acknowledge(&id)).await
    }

    /// Snoozes a message for `minutes` (default 15): local, never sent.
    ///
    /// # Errors
    ///
    /// [`HubError::MessageClosed`] for a closed message.
    pub async fn snooze(&self, id: MessageId, minutes: Option<u32>) -> Result<Message, HubError> {
        self.call(move |hub| hub.snooze(&id, minutes)).await
    }

    /// Clears a message without answering it.
    ///
    /// # Errors
    ///
    /// [`HubError::UseDoneInstead`] for `action_required`.
    pub async fn dismiss(&self, id: MessageId) -> Result<Message, HubError> {
        self.call(move |hub| hub.dismiss(&id)).await
    }

    /// Ticks or unticks a step: progress, never an answer.
    ///
    /// # Errors
    ///
    /// [`HubError::UnknownStep`].
    pub async fn tick_step(
        &self,
        id: MessageId,
        index: usize,
        is_ticked: bool,
    ) -> Result<Message, HubError> {
        self.call(move |hub| hub.tick_step(&id, index, is_ticked))
            .await
    }

    /// One message.
    ///
    /// # Errors
    ///
    /// [`HubError::MessageNotFound`].
    pub async fn message(&self, id: MessageId) -> Result<Message, HubError> {
        self.call(move |hub| Ok(hub.store().message(&id)?)).await
    }

    /// Messages matching a filter, newest first.
    ///
    /// # Errors
    ///
    /// Only the hub's own failures.
    pub async fn list_messages(&self, filter: MessageFilter) -> Result<Vec<Message>, HubError> {
        self.call(move |hub| Ok(hub.store().list_messages(&filter)?))
            .await
    }

    /// The buttons a message offers now, given its live waiter.
    ///
    /// # Errors
    ///
    /// [`HubError::MessageNotFound`].
    pub async fn offered_actions(&self, id: MessageId) -> Result<Vec<Action>, HubError> {
        self.call(move |hub| hub.offered_actions_of(&id)).await
    }

    /// Whether a message has a live waiter now.
    ///
    /// # Errors
    ///
    /// Only [`HubError::Stopped`].
    pub async fn has_live_waiter(&self, id: MessageId) -> Result<bool, HubError> {
        self.call(move |hub| Ok(hub.has_live_waiter(&id))).await
    }

    /// How many messages the hub keeps waiter state for: a diagnostic, and
    /// how tests check that closed messages are forgotten.
    ///
    /// # Errors
    ///
    /// Only [`HubError::Stopped`].
    pub async fn tracked_waiters(&self) -> Result<usize, HubError> {
        self.call(|hub| Ok(hub.tracked_waiters())).await
    }

    /// Mutes or unmutes a topic (and so its threads).
    ///
    /// # Errors
    ///
    /// [`HubError::TopicNotFound`].
    pub async fn mute_topic(&self, id: TopicId, is_muted: bool) -> Result<TopicRecord, HubError> {
        self.call(move |hub| hub.mute_topic(&id, is_muted)).await
    }

    /// Mutes or unmutes a thread.
    ///
    /// # Errors
    ///
    /// [`HubError::ThreadNotFound`].
    pub async fn mute_thread(
        &self,
        id: ThreadId,
        is_muted: bool,
    ) -> Result<ThreadRecord, HubError> {
        self.call(move |hub| hub.mute_thread(&id, is_muted)).await
    }

    /// "Mark all read" on a topic: acknowledges its open messages.
    ///
    /// # Errors
    ///
    /// [`HubError::TopicNotFound`].
    pub async fn mark_topic_read(&self, id: TopicId) -> Result<Vec<MessageId>, HubError> {
        self.call(move |hub| hub.mark_topic_read(&id)).await
    }

    /// "Mark all read" on a thread.
    ///
    /// # Errors
    ///
    /// [`HubError::ThreadNotFound`].
    pub async fn mark_thread_read(&self, id: ThreadId) -> Result<Vec<MessageId>, HubError> {
        self.call(move |hub| hub.mark_thread_read(&id)).await
    }

    /// Presence from the platform side (`PresenceUpdate`).
    ///
    /// # Errors
    ///
    /// Only [`HubError::Stopped`].
    pub async fn set_presence(&self, presence: Presence) -> Result<(), HubError> {
        self.call(move |hub| {
            hub.set_presence(presence);
            Ok(())
        })
        .await
    }

    /// Quiet hours started or ended.
    ///
    /// # Errors
    ///
    /// Only [`HubError::Stopped`].
    pub async fn set_quiet_hours(&self, is_active: bool) -> Result<(), HubError> {
        self.call(move |hub| {
            hub.set_quiet_hours(is_active);
            Ok(())
        })
        .await
    }

    /// Replaces the user's rules (a reloaded `rules.toml`).
    ///
    /// # Errors
    ///
    /// Only [`HubError::Stopped`].
    pub async fn set_rules(&self, rules: RuleSet) -> Result<(), HubError> {
        self.call(move |hub| {
            hub.set_rules(rules);
            Ok(())
        })
        .await
    }

    /// Replaces the policy table.
    ///
    /// # Errors
    ///
    /// Only [`HubError::Stopped`].
    pub async fn set_policy(&self, policy: Policy) -> Result<(), HubError> {
        self.call(move |hub| {
            hub.set_policy(policy);
            Ok(())
        })
        .await
    }

    /// Replaces the per-source strict modes.
    ///
    /// # Errors
    ///
    /// Only [`HubError::Stopped`].
    pub async fn set_source_settings(
        &self,
        sources: BTreeMap<String, IngestSettings>,
    ) -> Result<(), HubError> {
        self.call(move |hub| {
            hub.set_sources(sources);
            Ok(())
        })
        .await
    }

    /// Fires the timers that are due: the actor does this before every
    /// command and when it wakes, so this is for tests and for the app
    /// waking from sleep.
    ///
    /// # Errors
    ///
    /// Only [`HubError::Stopped`].
    pub async fn run_due_timers(&self) -> Result<(), HubError> {
        self.call(|_| Ok(())).await
    }

    /// Runs the retention sweep now.
    ///
    /// # Errors
    ///
    /// The store's failures.
    pub async fn sweep(&self) -> Result<SweepReport, HubError> {
        self.call(|hub| hub.sweep()).await
    }

    /// Stops the actor once the commands sent before this one are done; the
    /// store closes and the hub lock is released.
    ///
    /// # Errors
    ///
    /// [`HubError::Stopped`] when it already stopped.
    pub async fn shutdown(&self) -> Result<(), HubError> {
        self.call(|hub| {
            hub.stop();
            Ok(())
        })
        .await
    }
}
