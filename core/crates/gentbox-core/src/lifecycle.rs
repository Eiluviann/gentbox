//! Message lifecycle (architecture §5.2): the states, the allowed transitions
//! and what each button does to a stored message.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::{Duration, OffsetDateTime};

use crate::message::{ActionKind, Blocking, Message, MessageType};
use crate::rescind::RescindReason;
use crate::response::{AnsweredVia, ResponseStatus};

/// Default snooze when the action carries no minutes.
pub const DEFAULT_SNOOZE_MINUTES: u32 = 15;

/// Where a message is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleState {
    /// Delivered and needs attention: badge, may re-ping.
    Open,
    /// Seen: no re-ping, no badge, still answerable.
    Acknowledged,
    /// An answer action was taken.
    Answered,
    /// Cleared from the inbox without answering. Never an `action_required`
    /// message: those are closed with Done.
    Dismissed,
    /// Withdrawn by the sender, a supersede, a completion or progress.
    Rescinded,
    /// `expiresAt` passed.
    Expired,
}

impl LifecycleState {
    /// Every state.
    pub const ALL: [Self; 6] = [
        Self::Open,
        Self::Acknowledged,
        Self::Answered,
        Self::Dismissed,
        Self::Rescinded,
        Self::Expired,
    ];

    /// Open or acknowledged: the message can still be answered.
    pub fn is_live(self) -> bool {
        matches!(self, Self::Open | Self::Acknowledged)
    }

    /// A terminal state.
    pub fn is_closed(self) -> bool {
        !self.is_live()
    }
}

/// Why a message is closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "reason")]
pub enum ResolutionReason {
    /// An answer action was taken.
    Answered,
    /// Cleared without answering.
    Dismissed,
    /// Withdrawn, with the reason.
    Rescinded(RescindReason),
    /// Expired.
    Expired,
}

/// The state of a message plus the timestamps its transitions set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Lifecycle {
    /// Current state.
    pub state: LifecycleState,
    /// Hidden from the open list until this instant (acknowledged only).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "time::serde::rfc3339::option"
    )]
    pub snoozed_until: Option<OffsetDateTime>,
    /// When Defer was chosen.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "time::serde::rfc3339::option"
    )]
    pub deferred_at: Option<OffsetDateTime>,
    /// Why the message closed, once it has.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<ResolutionReason>,
    /// The answer that closed the message: what a later long-poll returns,
    /// including a Done tapped while no sender was waiting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<RecordedAnswer>,
}

/// The answer that closed a message, kept so that a sender that polls later
/// still hears it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordedAnswer {
    /// The pressed action's id.
    pub action_id: String,
    /// Its kind: approve, deny, choice, reply or done.
    pub action_kind: ActionKind,
    /// The choice value or reply text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// The channel it came through.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answered_via: Option<AnsweredVia>,
    /// When it was given.
    #[serde(with = "time::serde::rfc3339")]
    pub answered_at: OffsetDateTime,
}

impl Lifecycle {
    /// The lifecycle of a freshly stored message: `generic` is born
    /// acknowledged so it never badges, everything else open.
    pub fn born(message_type: MessageType) -> Self {
        let state = match message_type {
            MessageType::Generic => LifecycleState::Acknowledged,
            _ => LifecycleState::Open,
        };
        Self {
            state,
            snoozed_until: None,
            deferred_at: None,
            resolution: None,
            answer: None,
        }
    }
}

/// What can happen to a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleEvent {
    /// Seen: popup closed, inbox opened, conversation became frontmost.
    Acknowledge,
    /// Snoozed until the instant.
    Snooze {
        /// When the message returns to `open`.
        until: OffsetDateTime,
    },
    /// The snooze time passed.
    SnoozeEnded,
    /// Defer was chosen; legal on `deferrable` messages only (see [`defer`]).
    Defer,
    /// An answer action was taken.
    Answer,
    /// Cleared from the inbox without an answer (see [`dismiss`]).
    Dismiss,
    /// Withdrawn.
    Rescind(RescindReason),
    /// `expiresAt` passed.
    Expire,
}

impl LifecycleEvent {
    fn name(&self) -> &'static str {
        match self {
            Self::Acknowledge => "acknowledge",
            Self::Snooze { .. } => "snooze",
            Self::SnoozeEnded => "snooze_ended",
            Self::Defer => "defer",
            Self::Answer => "answer",
            Self::Dismiss => "dismiss",
            Self::Rescind(_) => "rescind",
            Self::Expire => "expire",
        }
    }
}

/// A rejected lifecycle change.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TransitionError {
    /// The event is not allowed in the current state.
    #[error("cannot {event} a message that is {from:?}")]
    NotAllowed {
        /// The state the message was in.
        from: LifecycleState,
        /// The rejected event.
        event: &'static str,
    },
}

/// The state machine of architecture §5.2.
///
/// `Defer` leaves the state unchanged (it is not a state); whether Defer is
/// offered at all depends on the blocking level and is checked by [`defer`].
pub fn transition(
    from: LifecycleState,
    event: &LifecycleEvent,
) -> Result<LifecycleState, TransitionError> {
    use LifecycleState::{Acknowledged, Answered, Dismissed, Expired, Open, Rescinded};
    let to = match (from, event) {
        (Open, LifecycleEvent::Acknowledge) => Acknowledged,
        (Open | Acknowledged, LifecycleEvent::Snooze { .. }) => Acknowledged,
        (Acknowledged, LifecycleEvent::SnoozeEnded) => Open,
        (Open | Acknowledged, LifecycleEvent::Defer) => from,
        (Open | Acknowledged, LifecycleEvent::Answer) => Answered,
        (Open | Acknowledged, LifecycleEvent::Dismiss) => Dismissed,
        (Open | Acknowledged, LifecycleEvent::Rescind(_)) => Rescinded,
        (Open | Acknowledged, LifecycleEvent::Expire) => Expired,
        _ => {
            return Err(TransitionError::NotAllowed {
                from,
                event: event.name(),
            });
        }
    };
    Ok(to)
}

/// Applies an event to a lifecycle, setting the timestamps and the
/// resolution that go with it.
pub fn apply_event(
    lifecycle: &Lifecycle,
    event: &LifecycleEvent,
    now: OffsetDateTime,
) -> Result<Lifecycle, TransitionError> {
    let state = transition(lifecycle.state, event)?;
    let mut next = lifecycle.clone();
    next.state = state;
    match event {
        LifecycleEvent::Snooze { until } => next.snoozed_until = Some(*until),
        LifecycleEvent::SnoozeEnded => next.snoozed_until = None,
        LifecycleEvent::Defer => next.deferred_at = Some(now),
        LifecycleEvent::Answer => next.resolution = Some(ResolutionReason::Answered),
        LifecycleEvent::Dismiss => next.resolution = Some(ResolutionReason::Dismissed),
        LifecycleEvent::Rescind(reason) => {
            next.resolution = Some(ResolutionReason::Rescinded(*reason))
        }
        LifecycleEvent::Expire => next.resolution = Some(ResolutionReason::Expired),
        LifecycleEvent::Acknowledge => {}
    }
    if state.is_closed() {
        next.snoozed_until = None;
    }
    Ok(next)
}

/// Why a button press was refused.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ActionError {
    /// The message is closed: a late answer to an answered, withdrawn,
    /// cleared or expired message ("Already answered", "Withdrawn").
    #[error("the message is already {state:?}")]
    Closed {
        /// The terminal state the message is in.
        state: LifecycleState,
        /// Why it closed.
        resolution: Option<ResolutionReason>,
    },
    /// Defer was pressed on a message that is not `deferrable`.
    #[error("only a deferrable message can be deferred")]
    NotDeferrable,
    /// The snooze duration does not fit a timestamp.
    #[error("the snooze duration is out of range")]
    SnoozeOutOfRange,
    /// An `action_required` message is closed with Done, never cleared.
    #[error("an action_required message is closed with Done")]
    UseDoneInstead,
    /// The pressed action id is not on the message, or has another kind.
    #[error("the message has no such action")]
    UnknownAction,
}

/// The effect of a button press on a stored message.
#[derive(Debug, Clone, PartialEq)]
pub struct ActionOutcome {
    /// The message after the press.
    pub message: Message,
    /// What the waiting sender is told. `None` for `open` and `snooze`, which
    /// never reach it.
    pub sender_status: Option<ResponseStatus>,
}

/// A button press: which action, and what it carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionPress {
    /// The pressed action's kind.
    pub kind: ActionKind,
    /// The pressed action's id. When given it must exist on the message with
    /// this kind; the answer is recorded under it.
    pub action_id: Option<String>,
    /// The choice value or the reply text.
    pub value: Option<String>,
    /// The channel the press came through.
    pub via: Option<AnsweredVia>,
    /// Snooze minutes (default 15).
    pub snooze_minutes: Option<u32>,
}

impl ActionPress {
    /// A press of the given kind with nothing else set.
    pub fn of_kind(kind: ActionKind) -> Self {
        Self {
            kind,
            action_id: None,
            value: None,
            via: None,
            snooze_minutes: None,
        }
    }
}

/// Applies a button press by kind: a thin wrapper over [`press_action`] that
/// records the answer under the message's first action of that kind.
pub fn take_action(
    message: &Message,
    kind: ActionKind,
    now: OffsetDateTime,
    snooze_minutes: Option<u32>,
) -> Result<ActionOutcome, ActionError> {
    let mut press = ActionPress::of_kind(kind);
    press.snooze_minutes = snooze_minutes;
    press_action(message, &press, now)
}

/// Applies a button press to a message.
///
/// - the answers (`approve`, `deny`, `choice`, `reply`, `done`) close the
///   message as `answered` and record the answer ([`RecordedAnswer`]) so a
///   later long-poll gets it. **Done closes an `action_required` message
///   locally whether or not a sender is waiting**: with a live waiter the
///   caller delivers it at once, without one the next poll finds it recorded;
/// - `defer` only on `deferrable`: the message turns `non_blocking` and stays
///   open (the caller re-runs the policy on the returned message);
/// - `open` acknowledges; `snooze` acknowledges with `snoozed_until = now +
///   minutes` (default 15). Neither is ever returned to the sender.
///
/// A message that is already closed refuses every action, so a rescinded
/// message rejects late answers and a late second answer loses. An answer or
/// Defer the message has no button for is [`ActionError::UnknownAction`]:
/// Approve on a completion or Done on a decision never closes anything. The
/// core does not know about live waiters: the hub checks
/// [`crate::actions::offered_actions`] before pressing.
pub fn press_action(
    message: &Message,
    press: &ActionPress,
    now: OffsetDateTime,
) -> Result<ActionOutcome, ActionError> {
    let state = message.lifecycle.state;
    if state.is_closed() {
        return Err(closed_error(message));
    }
    match press.kind {
        ActionKind::Defer => {
            if message.blocking != Blocking::Deferrable {
                return Err(ActionError::NotDeferrable);
            }
            pressed_action(message, press)?;
            defer(message, now)
        }
        ActionKind::Open => Ok(acknowledge_if_open(message, now)),
        ActionKind::Snooze => snooze(message, now, press.snooze_minutes),
        ActionKind::Approve
        | ActionKind::Deny
        | ActionKind::Choice
        | ActionKind::Reply
        | ActionKind::Done => answer(message, press, now),
    }
}

/// Clears a message from the inbox without answering it (`dismissed`).
///
/// An `action_required` message is never cleared: it is closed with Done,
/// which tells a waiting agent, or records the answer for a later poll.
pub fn dismiss(message: &Message, now: OffsetDateTime) -> Result<Message, ActionError> {
    if message.message_type == MessageType::ActionRequired {
        return Err(ActionError::UseDoneInstead);
    }
    let lifecycle = apply_event(&message.lifecycle, &LifecycleEvent::Dismiss, now)
        .map_err(|_| closed_error(message))?;
    let mut dismissed = message.clone();
    dismissed.lifecycle = lifecycle;
    Ok(dismissed)
}

/// Defer: legal on `deferrable` messages only. Records `deferred_at`, sets
/// `blocking` to `non_blocking` and leaves the message open or acknowledged.
pub fn defer(message: &Message, now: OffsetDateTime) -> Result<ActionOutcome, ActionError> {
    if message.blocking != Blocking::Deferrable {
        return Err(ActionError::NotDeferrable);
    }
    let lifecycle = apply_event(&message.lifecycle, &LifecycleEvent::Defer, now)
        .map_err(|_| closed_error(message))?;
    let mut deferred = message.clone();
    deferred.lifecycle = lifecycle;
    deferred.blocking = Blocking::NonBlocking;
    Ok(ActionOutcome {
        message: deferred,
        sender_status: Some(ResponseStatus::Deferred),
    })
}

fn answer(
    message: &Message,
    press: &ActionPress,
    now: OffsetDateTime,
) -> Result<ActionOutcome, ActionError> {
    let action_id = pressed_action(message, press)?.id.clone();
    let lifecycle = apply_event(&message.lifecycle, &LifecycleEvent::Answer, now)
        .map_err(|_| closed_error(message))?;
    let mut answered = message.clone();
    answered.lifecycle = lifecycle;
    answered.lifecycle.answer = Some(RecordedAnswer {
        action_id,
        action_kind: press.kind,
        value: press.value.clone(),
        answered_via: press.via,
        answered_at: now,
    });
    Ok(ActionOutcome {
        message: answered,
        sender_status: Some(ResponseStatus::Answered),
    })
}

/// The button a press stands for: the pressed id (which must exist with that
/// kind), else the message's first button of the kind. A kind the message has
/// no button for is refused; the hub adds Done to every `action_required`
/// message and Defer to every `deferrable` one, so those always exist.
fn pressed_action<'a>(
    message: &'a Message,
    press: &ActionPress,
) -> Result<&'a crate::message::Action, ActionError> {
    let mut of_kind = message
        .actions
        .iter()
        .filter(|action| action.kind == press.kind);
    match &press.action_id {
        Some(id) => of_kind.find(|action| &action.id == id),
        None => of_kind.next(),
    }
    .ok_or(ActionError::UnknownAction)
}

fn acknowledge_if_open(message: &Message, now: OffsetDateTime) -> ActionOutcome {
    let mut acknowledged = message.clone();
    if let Ok(lifecycle) = apply_event(&message.lifecycle, &LifecycleEvent::Acknowledge, now) {
        acknowledged.lifecycle = lifecycle;
    }
    ActionOutcome {
        message: acknowledged,
        sender_status: None,
    }
}

fn snooze(
    message: &Message,
    now: OffsetDateTime,
    snooze_minutes: Option<u32>,
) -> Result<ActionOutcome, ActionError> {
    let minutes = snooze_minutes.unwrap_or(DEFAULT_SNOOZE_MINUTES);
    let until = now
        .checked_add(Duration::minutes(i64::from(minutes)))
        .ok_or(ActionError::SnoozeOutOfRange)?;
    let lifecycle = apply_event(&message.lifecycle, &LifecycleEvent::Snooze { until }, now)
        .map_err(|_| closed_error(message))?;
    let mut snoozed = message.clone();
    snoozed.lifecycle = lifecycle;
    Ok(ActionOutcome {
        message: snoozed,
        sender_status: None,
    })
}

fn closed_error(message: &Message) -> ActionError {
    ActionError::Closed {
        state: message.lifecycle.state,
        resolution: message.lifecycle.resolution,
    }
}

/// How long a message lives when the sender gives no `expiresAt`:
/// `blocking` and `deferrable` 24 h, `non_blocking` decisions and actions
/// 7 days, `task_completed` 7 days, `generic` 24 h.
pub fn default_time_to_live(message_type: MessageType, blocking: Blocking) -> Duration {
    match (message_type, blocking) {
        (MessageType::Generic, _) => Duration::hours(24),
        (MessageType::TaskCompleted, _) => Duration::days(7),
        (_, Blocking::Blocking | Blocking::Deferrable) => Duration::hours(24),
        (_, Blocking::NonBlocking) => Duration::days(7),
    }
}
