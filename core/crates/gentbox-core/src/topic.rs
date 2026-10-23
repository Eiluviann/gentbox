//! Topics and threads (ADRs 0008, 0009): the grouping model, what a message
//! may name, and the wire records the hub returns.
//!
//! Gentbox topics are unrelated to ntfy topics and to the APNs `apns-topic`.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::ids::{ThreadId, TopicId};
use crate::rescind::RescindReason;

/// `kind` of the implicit per-agent General topic.
pub const GENERAL_TOPIC_KIND: &str = "general";
/// Title of the implicit General topic.
pub const GENERAL_TOPIC_TITLE: &str = "General";
/// `kind` of a conversation's implicit default thread, and of the explicit
/// thread that takes it over.
pub const CONVERSATION_THREAD_KIND: &str = "conversation";

/// Whether a topic or thread accepts new work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupStatus {
    /// Open.
    Open,
    /// Closed: shown in the Closed section until a message reopens it.
    Closed,
}

/// Colour hint from the app's palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(missing_docs)]
pub enum TopicColor {
    Blue,
    Purple,
    Pink,
    Red,
    Orange,
    Yellow,
    Green,
    Teal,
    Indigo,
    Gray,
}

/// SF Symbol and colour hints, shared with the topic's threads.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TopicStyle {
    /// SF Symbol name hint; shown only when on the app's allowed list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
    /// Colour hint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<TopicColor>,
}

/// `{ "id": … }`: an existing topic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TopicIdRef {
    /// The hub-assigned topic id.
    pub id: TopicId,
}

/// `{ "key": …, … }`: a topic by its stable key, created on first use. The
/// inline form never changes an existing topic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TopicKeyRef {
    /// Stable agent-chosen key, unique per hub across all agents.
    pub key: String,
    /// Title used on creation; without it the key becomes the title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Summary used on creation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Style used on creation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<TopicStyle>,
    /// Free-form hint; never changes behaviour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

/// What a message (or a thread create) may say about its topic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TopicRef {
    /// An existing topic.
    Id(TopicIdRef),
    /// A topic by key.
    Key(TopicKeyRef),
}

/// `{ "id": … }`: an existing thread (and so its topic).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThreadIdRef {
    /// The hub-assigned thread id.
    pub id: ThreadId,
}

/// `{ "key": …, … }`: a thread by key within the resolved topic, created on
/// first use. The inline form never takes over a conversation's implicit
/// thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThreadKeyRef {
    /// Stable agent-chosen key, unique within its topic.
    pub key: String,
    /// Title used on creation; without it the key becomes the title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Summary used on creation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Free-form hint; never changes behaviour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

/// What a message may say about its thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ThreadRef {
    /// An existing thread.
    Id(ThreadIdRef),
    /// A thread by key.
    Key(ThreadKeyRef),
}

/// Who created a topic or thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreatedBy {
    /// The agent.
    pub agent: String,
    /// The conversation, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
}

/// The conversation a thread is the default thread of.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DefaultFor {
    /// The agent.
    pub agent: String,
    /// The conversation.
    pub conversation_id: String,
}

/// A topic: a project, a bigger undertaking, or a category for simple
/// conversations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Topic {
    /// Hub id.
    pub id: TopicId,
    /// Stable key, unique per hub; none for the implicit General topic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// Title.
    pub title: String,
    /// Optional summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Free-form hint; never changes behaviour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Style hints, shared with its threads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<TopicStyle>,
    /// True for an agent's hub-managed General topic.
    pub implicit: bool,
    /// Open or closed.
    pub status: GroupStatus,
    /// Muted by you: non-urgent messages in it go to the inbox only.
    pub muted: bool,
    /// Last "mark all read".
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "time::serde::rfc3339::option"
    )]
    pub marked_read_at: Option<OffsetDateTime>,
    /// Who created it.
    pub created_by: CreatedBy,
    /// Creation time.
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// Last change.
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    /// When it was closed.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "time::serde::rfc3339::option"
    )]
    pub closed_at: Option<OffsetDateTime>,
}

impl Topic {
    /// Whether this is `agent`'s implicit General topic.
    pub fn is_general_topic_of(&self, agent: &str) -> bool {
        self.implicit
            && self.kind.as_deref() == Some(GENERAL_TOPIC_KIND)
            && self.created_by.agent == agent
    }
}

/// A thread within a topic: a version, a phase, or simply a conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    /// Hub id.
    pub id: ThreadId,
    /// The topic it belongs to.
    pub topic_id: TopicId,
    /// Stable key, unique within its topic; none for implicit threads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// Title.
    pub title: String,
    /// Optional summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Free-form hint; never changes behaviour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// True for a conversation's hub-managed default thread.
    pub implicit: bool,
    /// Open or closed.
    pub status: GroupStatus,
    /// Closed because its topic closed; reopening the topic reopens it.
    pub closed_by_topic: bool,
    /// Muted by you.
    pub muted: bool,
    /// Last "mark all read".
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "time::serde::rfc3339::option"
    )]
    pub marked_read_at: Option<OffsetDateTime>,
    /// The conversation whose default thread in this topic this is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_for: Option<DefaultFor>,
    /// Who created it.
    pub created_by: CreatedBy,
    /// Creation time.
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// Last change.
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    /// When it was closed.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "time::serde::rfc3339::option"
    )]
    pub closed_at: Option<OffsetDateTime>,
}

/// Muting a topic covers all its threads: a message is muted when its topic
/// or its thread is.
pub fn is_effectively_muted(topic: &Topic, thread: &Thread) -> bool {
    topic.muted || thread.muted
}

/// A topic as the hub returns it (`$defs/topicRecord`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TopicRecord {
    /// Hub id.
    pub id: TopicId,
    /// Stable key; none for implicit topics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// Title.
    pub title: String,
    /// Summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Kind hint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// True for the General topic.
    pub implicit: bool,
    /// Style hints.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<TopicStyle>,
    /// Open or closed.
    pub status: GroupStatus,
    /// Who created it.
    pub created_by: CreatedBy,
    /// Creation time.
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// Last change.
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    /// When it was closed.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "time::serde::rfc3339::option"
    )]
    pub closed_at: Option<OffsetDateTime>,
    /// Open and acknowledged messages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_messages: Option<u32>,
    /// Open threads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_threads: Option<u32>,
}

impl TopicRecord {
    /// The record the hub returns for a topic, with the counts it computed.
    pub fn from_topic(topic: &Topic, open_messages: u32, open_threads: u32) -> Self {
        Self {
            id: topic.id.clone(),
            key: topic.key.clone(),
            title: topic.title.clone(),
            summary: topic.summary.clone(),
            kind: topic.kind.clone(),
            implicit: topic.implicit,
            style: topic.style.clone(),
            status: topic.status,
            created_by: topic.created_by.clone(),
            created_at: topic.created_at,
            updated_at: topic.updated_at,
            closed_at: topic.closed_at,
            open_messages: Some(open_messages),
            open_threads: Some(open_threads),
        }
    }
}

/// A thread as the hub returns it (`$defs/threadRecord`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThreadRecord {
    /// Hub id.
    pub id: ThreadId,
    /// The topic it belongs to.
    pub topic_id: TopicId,
    /// Stable key; none for implicit threads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// Title.
    pub title: String,
    /// Summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Kind hint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// True for a conversation's default thread.
    pub implicit: bool,
    /// Open or closed.
    pub status: GroupStatus,
    /// The conversation it is the default thread of.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_for: Option<DefaultFor>,
    /// Who created it.
    pub created_by: CreatedBy,
    /// Creation time.
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// Last change.
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    /// When it was closed.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "time::serde::rfc3339::option"
    )]
    pub closed_at: Option<OffsetDateTime>,
    /// Open and acknowledged messages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_messages: Option<u32>,
    /// How many conversations have posted into it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversations: Option<u32>,
}

impl ThreadRecord {
    /// The record the hub returns for a thread, with the counts it computed.
    pub fn from_thread(thread: &Thread, open_messages: u32, conversations: u32) -> Self {
        Self {
            id: thread.id.clone(),
            topic_id: thread.topic_id.clone(),
            key: thread.key.clone(),
            title: thread.title.clone(),
            summary: thread.summary.clone(),
            kind: thread.kind.clone(),
            implicit: thread.implicit,
            status: thread.status,
            default_for: thread.default_for.clone(),
            created_by: thread.created_by.clone(),
            created_at: thread.created_at,
            updated_at: thread.updated_at,
            closed_at: thread.closed_at,
            open_messages: Some(open_messages),
            conversations: Some(conversations),
        }
    }
}

/// The conversation that creates a topic or thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreatingConversation {
    /// The conversation id.
    pub id: String,
}

/// Body of `POST /v1/topics` (`$defs/topicCreate`): an upsert by `key`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TopicCreate {
    /// Stable key.
    pub key: String,
    /// Title.
    pub title: String,
    /// Summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Kind hint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Style hints.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<TopicStyle>,
    /// The creating conversation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation: Option<CreatingConversation>,
}

/// Body of `POST /v1/threads` (`$defs/threadCreate`): an upsert by `key`
/// within the topic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThreadCreate {
    /// The topic; resolved like a message's when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic: Option<TopicRef>,
    /// Stable key within the topic.
    pub key: String,
    /// Title.
    pub title: String,
    /// Summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Kind hint; `conversation` with `conversation.id` takes over the
    /// conversation's implicit thread in the topic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// The creating conversation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation: Option<CreatingConversation>,
}

/// Body of `POST /v1/topics/{id}/close` and `POST /v1/threads/{id}/close`
/// (`$defs/closeRequest`). Closing never touches open messages unless
/// `rescind_open_messages` is true; then every open message in the topic or
/// thread is rescinded with `reason` (default `obsolete`). Closing a topic
/// also closes its open threads.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CloseRequest {
    /// Also withdraw the open messages.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub rescind_open_messages: bool,
    /// Why they are withdrawn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<RescindReason>,
}

impl CloseRequest {
    /// The reason open messages are withdrawn with: the given one, else
    /// `obsolete`.
    pub fn rescind_reason(&self) -> RescindReason {
        self.reason.unwrap_or(RescindReason::Obsolete)
    }
}
