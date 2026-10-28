//! Dispatch to channels: the UI delegate (the Mac app through FFI, or a log
//! in `gentboxd`) and an optional phone [`Relay`], which v1 leaves unset.
//!
//! The actor calls the delegate synchronously, on the hub thread, in order,
//! after the store has committed the change it reports. A delegate must not
//! block: the Mac app hops to its main actor, `gentboxd` writes a log line.

use std::fmt::Debug;
use std::sync::Arc;

use gentbox_core::ids::{MessageId, ThreadId, TopicId};
use gentbox_core::item::DeliveredSurfaces;
use gentbox_core::lifecycle::LifecycleState;
use gentbox_core::message::{Action, Message};
use gentbox_core::policy::DeliveryPlan;
use gentbox_core::rescind::Rescission;
use gentbox_relay::Relay;

/// Something the surfaces must show or take down.
#[derive(Debug, Clone, PartialEq)]
pub enum HubEvent {
    /// A new message to show, or one that takes another's slot in place.
    Delivered {
        /// The stored message.
        message: Message,
        /// Where and how loud.
        plan: DeliveryPlan,
        /// The buttons to show now.
        offered_actions: Vec<Action>,
        /// Whether a sender is waiting for the answer.
        has_live_waiter: bool,
        /// The message whose popup and notification it takes over (same
        /// notification identifier), when it superseded one.
        replaces: Option<MessageId>,
        /// Whether it makes a sound: a supersede pings only when louder.
        pings: bool,
    },
    /// A still-open message pings again (re-ping, or the end of a snooze).
    Repinged {
        /// The message.
        message_id: MessageId,
        /// The plan, re-decided now.
        plan: DeliveryPlan,
    },
    /// A message absorbed a repeat (a fold or a dedupe); nothing pings.
    Repeated {
        /// The message that absorbed it.
        message_id: MessageId,
        /// Its repeat counter now.
        repeat_count: u32,
    },
    /// A live message changed without a new delivery: acknowledged, snoozed,
    /// deferred, steps ticked, moved.
    Updated {
        /// The message now.
        message: Message,
        /// The buttons to show now.
        offered_actions: Vec<Action>,
        /// Whether a sender is waiting for the answer now.
        has_live_waiter: bool,
        /// Surfaces to take down (Defer closes the popup).
        take_down: DeliveredSurfaces,
    },
    /// A message closed: answered, dismissed or expired. Take down what shows
    /// it; the inbox row stays with its new state.
    Closed {
        /// The message.
        message_id: MessageId,
        /// Its terminal state.
        state: LifecycleState,
        /// Surfaces to take down.
        take_down: DeliveredSurfaces,
    },
    /// A message was withdrawn, with what each channel must do.
    Withdrawn(Rescission),
    /// The live waiter appeared or lapsed: answer buttons appear or go.
    WaiterChanged {
        /// The message.
        message_id: MessageId,
        /// Whether a sender is waiting now.
        has_live_waiter: bool,
        /// The buttons to show now.
        offered_actions: Vec<Action>,
    },
    /// A topic was created, changed, closed, reopened, muted or read.
    TopicChanged(TopicId),
    /// A thread was created, changed, closed, reopened, moved, muted or read.
    ThreadChanged(ThreadId),
}

/// Receives what the surfaces must do.
///
/// [`HubDelegate::on_event`] runs **on the hub thread**, inside the actor's
/// turn, so:
///
/// - **Never wait on a [`crate::HubHandle`] call from it.** The actor is busy
///   delivering this event and cannot run the command, so blocking on its
///   reply (`block_on`, a semaphore, a synchronous FFI round trip) deadlocks
///   the hub.
/// - **`tokio::spawn` here lands on the hub's runtime**, a single-threaded
///   one that also runs the actor and its timers. Anything spawned there
///   competes with every command; hand work to a runtime or queue of your own
///   instead (the Mac app hops to its main actor and returns at once).
/// - Return quickly and do not panic: events after this one wait for it.
pub trait HubDelegate: Send + Sync + Debug {
    /// Called on the hub thread, in order, after the change is stored. Must
    /// not block and must not wait on the hub (see the trait docs).
    fn on_event(&self, event: HubEvent);
}

/// A delegate that ignores everything.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoDelegate;

impl HubDelegate for NoDelegate {
    fn on_event(&self, _event: HubEvent) {}
}

/// The channels a hub dispatches to.
#[derive(Clone)]
pub(crate) struct Dispatcher {
    delegate: Arc<dyn HubDelegate>,
    /// Held for M1; v1 never calls it (see `has_phone_channel`).
    relay: Option<Arc<dyn Relay>>,
}

impl Debug for Dispatcher {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Dispatcher")
            .field("delegate", &self.delegate)
            .field("relay", &self.relay.as_ref().map(|relay| relay.name()))
            .finish()
    }
}

impl Dispatcher {
    pub(crate) fn new(delegate: Arc<dyn HubDelegate>, relay: Option<Arc<dyn Relay>>) -> Self {
        Self { delegate, relay }
    }

    /// Whether a phone channel exists. Always false in v1: the policy has no
    /// phone columns and nothing publishes to a relay yet, so a relay passed
    /// to `Hub::start` is held for the mobile release (M1) and never used,
    /// and rescinds list no phone effects.
    pub(crate) fn has_phone_channel(&self) -> bool {
        false
    }

    pub(crate) fn send(&self, event: HubEvent) {
        self.delegate.on_event(event);
    }
}
