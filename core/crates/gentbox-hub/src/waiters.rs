//! Live waiters (architecture §5.2): a message has one while a long-poll for
//! it is in flight or ended less than 10 s ago.
//!
//! Each message with waiter activity gets a `tokio::sync::watch` channel
//! carrying its current [`ActionResponse`]. The actor publishes into it on
//! every change a sender hears about (answer, Defer, rescind, dismiss,
//! expiry); `open`, `snooze` and step ticks never touch it. A long-poll
//! subscribes, waits for a status other than `pending` or for its deadline,
//! and reports back through a guard that fires even when the HTTP client
//! hangs up.
//!
//! **Deferred once.** Each Defer bumps the message's deferral count, which
//! the watch carries next to the response. A poll remembers the count it
//! started with, so **every poll in flight when you defer** answers
//! `deferred`, whatever is published after. Defer also sets
//! `deferral_unreported`; it is cleared only when a poll actually hands
//! `deferred` over (one dropped by a vanished client does not count), and
//! while it is set the next poll to start answers `deferred` at once. After
//! that, polls wait for the real answer.

use std::collections::HashMap;
use std::time::Duration as StdDuration;

use gentbox_core::ids::MessageId;
use gentbox_core::response::{ActionResponse, ResponseStatus};
use time::{Duration, OffsetDateTime};
use tokio::sync::{mpsc, watch};

/// A long-poll counts as a live waiter until this long after it ended: the
/// gap between two polls.
pub const LIVE_WAITER_WINDOW: Duration = Duration::seconds(10);

/// The longest a single long-poll holds (`wait` is capped at 55 s).
pub const MAX_WAIT: StdDuration = StdDuration::from_secs(55);

/// What a long-poll tells the actor when it ends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WaiterSignal {
    /// The poll ended, normally or because its client went away.
    Ended {
        /// The message polled for.
        message_id: MessageId,
        /// It handed `deferred` to the sender.
        reported_deferral: bool,
    },
}

/// The start of a long-poll.
#[derive(Debug)]
pub enum WaitStart {
    /// The answer is already known: answered, deferred (reported now),
    /// dismissed, rescinded, expired or gone.
    Ready(ActionResponse),
    /// Still pending: wait for a change.
    Waiting(PendingWait),
}

impl WaitStart {
    /// Waits up to `timeout` (capped at [`MAX_WAIT`]) for the response.
    pub async fn wait(self, timeout: StdDuration) -> ActionResponse {
        match self {
            Self::Ready(response) => response,
            Self::Waiting(pending) => pending.wait(timeout).await,
        }
    }
}

/// What the watch carries: the response and how many times Defer was chosen.
#[derive(Debug, Clone)]
struct Published {
    response: ActionResponse,
    deferrals: u64,
}

/// A long-poll in flight. While it exists the message has a live waiter;
/// dropping it ends the poll.
#[derive(Debug)]
pub struct PendingWait {
    message_id: MessageId,
    receiver: watch::Receiver<Published>,
    deferrals_at_start: u64,
    guard: WaitGuard,
}

impl PendingWait {
    /// Waits up to `timeout` (capped at [`MAX_WAIT`]) for a status other
    /// than `pending`; at the deadline it answers `pending`.
    pub async fn wait(mut self, timeout: StdDuration) -> ActionResponse {
        let deadline = tokio::time::Instant::now() + timeout.min(MAX_WAIT);
        loop {
            tokio::select! {
                changed = self.receiver.changed() => {
                    if changed.is_err() {
                        // The hub stopped: the sender polls again and learns it.
                        return pending_response(&self.message_id);
                    }
                    if let Some(response) = self.settled() {
                        return response;
                    }
                }
                () = tokio::time::sleep_until(deadline) => {
                    return self.settled().unwrap_or_else(|| pending_response(&self.message_id));
                }
            }
        }
    }

    /// A terminal response, else `deferred` when Defer was chosen since the
    /// poll started, else nothing yet.
    fn settled(&mut self) -> Option<ActionResponse> {
        let published = self.receiver.borrow_and_update().clone();
        let is_terminal = !matches!(
            published.response.status,
            ResponseStatus::Pending | ResponseStatus::Deferred
        );
        if is_terminal {
            return Some(published.response);
        }
        if published.deferrals > self.deferrals_at_start {
            self.guard.reported_deferral = true;
            let mut deferred = pending_response(&self.message_id);
            deferred.status = ResponseStatus::Deferred;
            return Some(deferred);
        }
        None
    }
}

/// Tells the actor the poll ended, however it ended.
#[derive(Debug)]
struct WaitGuard {
    message_id: MessageId,
    signals: mpsc::UnboundedSender<WaiterSignal>,
    reported_deferral: bool,
}

impl Drop for WaitGuard {
    fn drop(&mut self) {
        // A closed channel means the hub stopped; nothing is left to tell.
        let _ = self.signals.send(WaiterSignal::Ended {
            message_id: self.message_id.clone(),
            reported_deferral: self.reported_deferral,
        });
    }
}

/// `{ messageId, status: pending }`.
pub(crate) fn pending_response(message_id: &MessageId) -> ActionResponse {
    ActionResponse {
        message_id: message_id.clone(),
        status: ResponseStatus::Pending,
        rescind_reason: None,
        action_id: None,
        action_kind: None,
        value: None,
        answered_via: None,
        answered_at: None,
    }
}

#[derive(Debug)]
struct WaiterState {
    sender: watch::Sender<Published>,
    in_flight: u32,
    last_poll_end: Option<OffsetDateTime>,
    deferrals: u64,
    deferral_unreported: bool,
    announced_live: bool,
}

impl WaiterState {
    fn new(message_id: &MessageId) -> Self {
        Self {
            sender: watch::Sender::new(Published {
                response: pending_response(message_id),
                deferrals: 0,
            }),
            in_flight: 0,
            last_poll_end: None,
            deferrals: 0,
            deferral_unreported: false,
            announced_live: false,
        }
    }

    fn publish(&self, response: ActionResponse) {
        self.sender.send_replace(Published {
            response,
            deferrals: self.deferrals,
        });
    }

    fn is_live(&self, now: OffsetDateTime) -> bool {
        self.in_flight > 0
            || self
                .last_poll_end
                .is_some_and(|ended| now - ended < LIVE_WAITER_WINDOW)
    }
}

/// Every message's waiter state. Owned by the actor.
#[derive(Debug)]
pub(crate) struct Waiters {
    by_message: HashMap<MessageId, WaiterState>,
    signals: mpsc::UnboundedSender<WaiterSignal>,
}

impl Waiters {
    pub(crate) fn new(signals: mpsc::UnboundedSender<WaiterSignal>) -> Self {
        Self {
            by_message: HashMap::new(),
            signals,
        }
    }

    /// How many long-polls for the message are in flight.
    pub(crate) fn in_flight(&self, id: &MessageId) -> u32 {
        self.by_message.get(id).map_or(0, |state| state.in_flight)
    }

    /// How many messages have waiter state.
    pub(crate) fn len(&self) -> usize {
        self.by_message.len()
    }

    /// Whether the message has a live waiter at `now`.
    pub(crate) fn is_live(&self, id: &MessageId, now: OffsetDateTime) -> bool {
        self.by_message
            .get(id)
            .is_some_and(|state| state.is_live(now))
    }

    /// Whether Defer was chosen and no sender has heard it yet.
    pub(crate) fn deferral_unreported(&self, id: &MessageId) -> bool {
        self.by_message
            .get(id)
            .is_some_and(|state| state.deferral_unreported)
    }

    /// A new message whose sender will poll: it counts as having a live
    /// waiter from the start, as if a poll had just ended, so its first poll
    /// has the usual 10 s to arrive. Returns when that lapses. Only for a new
    /// message: its Delivered event already shows the answer buttons, so the
    /// waiter counts as announced.
    pub(crate) fn expect(&mut self, id: &MessageId, now: OffsetDateTime) -> OffsetDateTime {
        let state = self
            .by_message
            .entry(id.clone())
            .or_insert_with(|| WaiterState::new(id));
        state.last_poll_end = Some(now);
        state.announced_live = true;
        now + LIVE_WAITER_WINDOW
    }

    /// A poll that just ended without being in flight: one answered at once
    /// (with `deferred`), or a repeat whose sender will poll. Returns when the
    /// waiter lapses; the caller announces the change.
    pub(crate) fn polled(&mut self, id: &MessageId, now: OffsetDateTime) -> OffsetDateTime {
        let state = self
            .by_message
            .entry(id.clone())
            .or_insert_with(|| WaiterState::new(id));
        state.last_poll_end = Some(now);
        now + LIVE_WAITER_WINDOW
    }

    /// Starts a long-poll on a pending message.
    pub(crate) fn begin(&mut self, current: ActionResponse) -> PendingWait {
        let id = current.message_id.clone();
        let state = self
            .by_message
            .entry(id.clone())
            .or_insert_with(|| WaiterState::new(&id));
        state.in_flight += 1;
        state.publish(current);
        PendingWait {
            receiver: state.sender.subscribe(),
            deferrals_at_start: state.deferrals,
            guard: WaitGuard {
                message_id: id.clone(),
                signals: self.signals.clone(),
                reported_deferral: false,
            },
            message_id: id,
        }
    }

    /// A long-poll ended. Returns when the waiter lapses if no other poll
    /// starts; a poll that handed `deferred` over clears the deferral.
    pub(crate) fn end(
        &mut self,
        id: &MessageId,
        now: OffsetDateTime,
        reported_deferral: bool,
    ) -> Option<OffsetDateTime> {
        let state = self.by_message.get_mut(id)?;
        state.in_flight = state.in_flight.saturating_sub(1);
        state.last_poll_end = Some(now);
        if reported_deferral {
            state.deferral_unreported = false;
        }
        Some(now + LIVE_WAITER_WINDOW)
    }

    /// Publishes a change the sender hears about.
    pub(crate) fn publish(&mut self, response: ActionResponse) {
        if let Some(state) = self.by_message.get(&response.message_id) {
            state.publish(response);
        }
    }

    /// Defer was chosen: every poll in flight answers `deferred`, and so does
    /// the next one to start until one has handed it over.
    pub(crate) fn defer(&mut self, response: ActionResponse) {
        let id = response.message_id.clone();
        let state = self
            .by_message
            .entry(id.clone())
            .or_insert_with(|| WaiterState::new(&id));
        state.deferrals += 1;
        state.deferral_unreported = true;
        state.publish(response);
    }

    /// A poll that started was handed `deferred` at once: reported.
    pub(crate) fn deferral_reported(&mut self, id: &MessageId) {
        if let Some(state) = self.by_message.get_mut(id) {
            state.deferral_unreported = false;
            state.publish(pending_response(id));
        }
    }

    /// When the live flag changed since it was last announced, the new
    /// value; the UI shows or hides the answer buttons on it.
    pub(crate) fn take_live_change(&mut self, id: &MessageId, now: OffsetDateTime) -> Option<bool> {
        let state = self.by_message.get_mut(id)?;
        let is_live = state.is_live(now);
        if is_live == state.announced_live {
            return None;
        }
        state.announced_live = is_live;
        Some(is_live)
    }

    /// Forgets a closed message once no poll is in flight and its waiter has
    /// lapsed; its last response stays readable from the store.
    pub(crate) fn forget_if_idle(&mut self, id: &MessageId, now: OffsetDateTime) {
        if self
            .by_message
            .get(id)
            .is_some_and(|state| !state.is_live(now))
        {
            self.by_message.remove(id);
        }
    }
}
