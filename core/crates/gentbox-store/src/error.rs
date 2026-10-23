//! What the store reports. Not-found and constraint outcomes are values the
//! caller handles; broken invariants (a caller handing over a message whose
//! topic is not its thread's topic) panic instead.

use gentbox_core::ids::{MessageId, ThreadId, TopicId};
use thiserror::Error;

/// The kind of record a lookup missed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityKind {
    /// A message.
    Message,
    /// A topic.
    Topic,
    /// A thread.
    Thread,
    /// A conversation.
    Conversation,
}

/// A uniqueness rule the write would have broken.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Conflict {
    /// A message with this id is stored.
    #[error("message {0} already exists")]
    MessageId(MessageId),
    /// This agent already used this idempotency key.
    #[error("idempotency key `{key}` was already used by `{agent}`")]
    IdempotencyKey {
        /// `source.agent`.
        agent: String,
        /// The sender's key.
        key: String,
    },
    /// A topic with this id is stored.
    #[error("topic {0} already exists")]
    TopicId(TopicId),
    /// A topic with this key exists (keys are unique per hub).
    #[error("topic key `{0}` is taken")]
    TopicKey(String),
    /// The agent already has its implicit General topic.
    #[error("agent `{0}` already has a General topic")]
    GeneralTopic(String),
    /// A thread with this id is stored.
    #[error("thread {0} already exists")]
    ThreadId(ThreadId),
    /// The topic already has a thread with this key.
    #[error("thread key `{key}` is taken in topic {topic_id}")]
    ThreadKey {
        /// The topic.
        topic_id: TopicId,
        /// The thread key.
        key: String,
    },
    /// The conversation already has a default thread in the topic.
    #[error(
        "conversation `{conversation_id}` of `{agent}` already has a default thread in topic {topic_id}"
    )]
    DefaultThread {
        /// The agent.
        agent: String,
        /// The conversation.
        conversation_id: String,
        /// The topic.
        topic_id: TopicId,
    },
    /// The thread is already some conversation's default thread.
    #[error("thread {0} is already a default thread")]
    ThreadIsDefault(ThreadId),
}

/// Why a store call failed.
#[derive(Debug, Error)]
pub enum StoreError {
    /// The record does not exist.
    #[error("{entity:?} `{id}` not found")]
    NotFound {
        /// What was looked up.
        entity: EntityKind,
        /// The id (or key) that missed.
        id: String,
    },
    /// The write would break a uniqueness rule.
    #[error(transparent)]
    Conflict(#[from] Conflict),
    /// A constraint the schema enforces that has no dedicated outcome; the
    /// store's own checks make this unreachable for well-formed calls.
    #[error("constraint violated: {0}")]
    Constraint(String),
    /// A timestamp does not fit the store's nanosecond column (about the years
    /// 1678 to 2262).
    #[error("timestamp out of range for the store")]
    TimestampOutOfRange,
    /// Stored data could not be decoded: the file was edited or written by
    /// something else.
    #[error("corrupt data in `{column}`: {detail}")]
    Corrupt {
        /// The column.
        column: &'static str,
        /// What was wrong.
        detail: String,
    },
    /// The database file comes from a newer Gentbox.
    #[error("database schema version {found} is newer than the {supported} this build supports")]
    SchemaTooNew {
        /// `user_version` of the file.
        found: u32,
        /// The newest version this build knows.
        supported: u32,
    },
    /// SQLite failed (IO, locking, a full disk).
    #[error("database error: {0}")]
    Database(#[source] rusqlite::Error),
}

impl StoreError {
    pub(crate) fn not_found(entity: EntityKind, id: impl ToString) -> Self {
        Self::NotFound {
            entity,
            id: id.to_string(),
        }
    }

    pub(crate) fn corrupt(column: &'static str, detail: impl ToString) -> Self {
        Self::Corrupt {
            column,
            detail: detail.to_string(),
        }
    }
}

impl From<rusqlite::Error> for StoreError {
    fn from(error: rusqlite::Error) -> Self {
        match error {
            rusqlite::Error::SqliteFailure(failure, message)
                if failure.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Self::Constraint(message.unwrap_or_else(|| failure.to_string()))
            }
            other => Self::Database(other),
        }
    }
}
