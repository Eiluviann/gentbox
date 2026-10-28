//! [`GentboxError`]: what Swift sees when a query or command fails.

use gentbox_hub::{HubError, StartError};
use gentbox_ingest::IngestStartError;

use gentbox_core::lifecycle::ResolutionReason;

use crate::records::{LifecycleState, RescindReason};

/// Why a call failed. `AnotherHubRunning`, `NotFound`, `AlreadyClosed` and
/// `Refused` are expected outcomes the UI words for you; `Stopped` and
/// `Internal` mean the hub is gone or broken.
#[derive(Debug, Clone, PartialEq, thiserror::Error, uniffi::Error)]
pub enum GentboxError {
    /// Another hub holds the state folder (`gentboxd`, or a second build).
    #[error("{message}")]
    AnotherHubRunning {
        /// The lock error, naming the holder.
        message: String,
    },
    /// The hub could not start: the store, a listener or a file failed.
    #[error("cannot start the hub: {message}")]
    CannotStart {
        /// What failed.
        message: String,
    },
    /// The message, topic or thread no longer exists.
    #[error("{message}")]
    NotFound {
        /// Which one.
        message: String,
    },
    /// The message closed before this answer: "Already answered".
    #[error("the message is already {state:?}")]
    AlreadyClosed {
        /// Its terminal state.
        state: LifecycleState,
        /// Why it was withdrawn, when `state` is `Rescinded`: "Withdrawn by
        /// the agent", or "Handled in the session" for the hub's clean-up.
        rescind_reason: Option<RescindReason>,
    },
    /// The hub refused the command: an action not offered now, Done
    /// required instead of dismiss, a key conflict on a move.
    #[error("{message}")]
    Refused {
        /// Why, in the hub's words.
        message: String,
    },
    /// A topic or thread closes only when nothing in it is open.
    #[error("{count} item(s) still open")]
    HasOpenMessages {
        /// How many messages are open in it.
        count: u32,
    },
    /// An agent's General topic never closes; its threads do.
    #[error("the General topic cannot be closed")]
    GeneralTopic,
    /// The hub has stopped.
    #[error("the hub has stopped")]
    Stopped,
    /// The store or the hub failed; the details are in the log.
    #[error("{message}")]
    Internal {
        /// What failed.
        message: String,
    },
}

impl From<HubError> for GentboxError {
    fn from(error: HubError) -> Self {
        match error {
            HubError::MessageNotFound(_)
            | HubError::TopicNotFound(_)
            | HubError::ThreadNotFound(_) => Self::NotFound {
                message: error.to_string(),
            },
            HubError::MessageClosed { state, resolution } => Self::AlreadyClosed {
                state: state.into(),
                rescind_reason: match resolution {
                    Some(ResolutionReason::Rescinded(reason)) => Some(reason.into()),
                    _ => None,
                },
            },
            HubError::HasOpenMessages { count } => Self::HasOpenMessages { count },
            HubError::GeneralTopic => Self::GeneralTopic,
            HubError::Stopped => Self::Stopped,
            HubError::Store(_) | HubError::Panicked => Self::Internal {
                message: error.to_string(),
            },
            other => Self::Refused {
                message: other.to_string(),
            },
        }
    }
}

impl From<StartError> for GentboxError {
    fn from(error: StartError) -> Self {
        match error {
            StartError::Lock(lock) => Self::AnotherHubRunning {
                message: lock.to_string(),
            },
            other => Self::CannotStart {
                message: other.to_string(),
            },
        }
    }
}

impl From<IngestStartError> for GentboxError {
    fn from(error: IngestStartError) -> Self {
        Self::CannotStart {
            message: error.to_string(),
        }
    }
}
