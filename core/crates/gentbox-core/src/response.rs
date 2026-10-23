//! What the hub tells senders: the long-poll `actionResponse` and the
//! `ingestResponse` of `POST /v1/messages`.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::classify::FieldSources;
use crate::ids::{MessageId, ThreadId, TopicId};
use crate::lifecycle::{LifecycleState, ResolutionReason};
use crate::message::{ActionKind, Blocking, Message, MessageType};
use crate::rescind::RescindReason;

/// `actionResponse.status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseStatus {
    /// Open (or snoozed): poll again.
    Pending,
    /// An answer was taken.
    Answered,
    /// Defer was chosen: keep working on what needs no input.
    Deferred,
    /// Cleared without answering (never an `action_required` message, which
    /// is closed with Done and reports `answered`).
    Dismissed,
    /// Withdrawn.
    Rescinded,
    /// Expired.
    Expired,
}

/// The status a long-poll reports for a message in `state`.
///
/// `deferral_unreported` is true between Defer being chosen and the waiter
/// hearing about it: the waiter gets `deferred` once, `pending` after, until
/// the real answer. `open`, `acknowledged` and snoozed messages are
/// `pending`; `open` and `snooze` are never returned.
pub fn response_status(state: LifecycleState, deferral_unreported: bool) -> ResponseStatus {
    match state {
        LifecycleState::Open | LifecycleState::Acknowledged if deferral_unreported => {
            ResponseStatus::Deferred
        }
        LifecycleState::Open | LifecycleState::Acknowledged => ResponseStatus::Pending,
        LifecycleState::Answered => ResponseStatus::Answered,
        LifecycleState::Dismissed => ResponseStatus::Dismissed,
        LifecycleState::Rescinded => ResponseStatus::Rescinded,
        LifecycleState::Expired => ResponseStatus::Expired,
    }
}

/// Which channel an answer came through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnsweredVia {
    /// The Mac app.
    Mac,
    /// The phone.
    Phone,
    /// The Watch.
    Watch,
    /// The CLI.
    Cli,
}

/// Body of `GET /v1/messages/{id}/response`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActionResponse {
    /// The hub-assigned message id.
    pub message_id: MessageId,
    /// Where the message stands.
    pub status: ResponseStatus,
    /// Why it was withdrawn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rescind_reason: Option<RescindReason>,
    /// The answering action's id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_id: Option<String>,
    /// The answering action's kind; only answer kinds are returned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_kind: Option<ActionKind>,
    /// The choice value or reply text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// The channel that answered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answered_via: Option<AnsweredVia>,
    /// When it was answered.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "time::serde::rfc3339::option"
    )]
    pub answered_at: Option<OffsetDateTime>,
}

impl ActionResponse {
    /// The long-poll response for a stored message.
    ///
    /// An answered message reports the recorded answer, so a sender that polls
    /// after a waiter-less Done (a `StopFailure` item you finished at the Mac)
    /// still gets `answered` with `actionKind: done`. `deferral_unreported` is
    /// the hub's "Defer chosen, waiter not told yet" flag.
    pub fn for_message(message: &Message, deferral_unreported: bool) -> Self {
        let lifecycle = &message.lifecycle;
        let status = response_status(lifecycle.state, deferral_unreported);
        let rescind_reason = match lifecycle.resolution {
            Some(ResolutionReason::Rescinded(reason)) => Some(reason),
            _ => None,
        };
        let answer = match status {
            ResponseStatus::Answered => lifecycle.answer.as_ref(),
            _ => None,
        };
        Self {
            message_id: message.id.clone(),
            status,
            rescind_reason,
            action_id: answer.map(|answer| answer.action_id.clone()),
            action_kind: answer.map(|answer| answer.action_kind),
            value: answer.and_then(|answer| answer.value.clone()),
            answered_via: answer.and_then(|answer| answer.answered_via),
            answered_at: answer.map(|answer| answer.answered_at),
        }
    }
}

/// What the hub changed, reported back to the sender.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WarningCode {
    /// The title looked like raw input and was rewritten.
    TitleLooksLikeCommand,
    /// The summary looked like raw input and was moved to the body.
    SummaryLooksLikeCommand,
    /// The conversation title looked like raw input and was ignored.
    ConversationTitleIgnored,
    /// A field was changed to make the three fields consistent.
    Normalized,
    /// An `action_required` message had no steps; a fallback was added.
    StepsMissing,
    /// A step's text looked like raw input and moved into its detail.
    StepRewritten,
    /// A secret was redacted.
    SecretRedacted,
}

/// One entry of `ingestResponse.warnings`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IngestWarning {
    /// What happened.
    pub code: WarningCode,
    /// The field it happened to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    /// Plain-words explanation for the sender (and the model behind it).
    pub message: String,
}

impl IngestWarning {
    /// Builds a warning for a field.
    pub fn new(code: WarningCode, field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code,
            field: Some(field.into()),
            message: message.into(),
        }
    }
}

/// Body of the 201 (or idempotent 200) response to `POST /v1/messages`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IngestResponse {
    /// The hub-assigned message id.
    pub id: MessageId,
    /// Final type.
    #[serde(rename = "type")]
    pub message_type: MessageType,
    /// Final urgency.
    pub urgent: bool,
    /// Final blocking level.
    pub blocking: Blocking,
    /// Where each field came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field_sources: Option<FieldSources>,
    /// The topic the message landed in.
    pub topic_id: TopicId,
    /// The thread the message landed in.
    pub thread_id: ThreadId,
    /// `open` or `acknowledged`.
    pub state: LifecycleState,
    /// Whether long-polling is worthwhile.
    pub should_wait: bool,
    /// What the hub changed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<IngestWarning>,
}
