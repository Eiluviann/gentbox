//! The callback interface: [`HubDelegate`], implemented in Swift, and the
//! [`HubEvent`]s it receives, converted from the hub's.

use std::fmt;
use std::sync::Arc;

use gentbox_hub::HubEvent as CoreHubEvent;

use crate::records::{
    DeliveryPlan, InboxAction, InboxMessage, LifecycleState, RescindReason, Surfaces,
};

/// Something the surfaces must show or take down. The inbox re-reads
/// [`crate::GentboxHub::inbox`] on any of them; popups and notifications
/// (PR 4) act on the details.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum HubEvent {
    /// A new message, or one that takes another's slot in place.
    Delivered {
        /// The message.
        message: InboxMessage,
        /// Where and how loud.
        plan: DeliveryPlan,
        /// The message whose popup and notification it takes over.
        replaces: Option<String>,
        /// Whether it makes a sound.
        pings: bool,
    },
    /// A still-open message pings again.
    Repinged {
        /// The message.
        message_id: String,
        /// The plan, re-decided now.
        plan: DeliveryPlan,
    },
    /// A message absorbed a repeat; nothing pings.
    Repeated {
        /// The message.
        message_id: String,
        /// Its repeat counter now.
        repeat_count: u32,
    },
    /// A live message changed without a new delivery.
    Updated {
        /// The message now.
        message: InboxMessage,
        /// Surfaces to take down.
        take_down: Surfaces,
    },
    /// A message closed: answered, dismissed or expired.
    Closed {
        /// The message.
        message_id: String,
        /// Its terminal state.
        state: LifecycleState,
        /// Surfaces to take down.
        take_down: Surfaces,
    },
    /// A message was withdrawn.
    Withdrawn {
        /// The message.
        message_id: String,
        /// Why, for the "Withdrawn" line.
        reason: RescindReason,
    },
    /// The live waiter appeared or lapsed: answer buttons appear or go.
    WaiterChanged {
        /// The message.
        message_id: String,
        /// Whether a sender waits now.
        has_live_waiter: bool,
        /// The buttons to show now.
        offered_actions: Vec<InboxAction>,
    },
    /// A topic changed: created, edited, closed, reopened, muted or read.
    TopicChanged {
        /// The topic.
        topic_id: String,
    },
    /// A thread changed: created, edited, closed, reopened, moved, muted or read.
    ThreadChanged {
        /// The thread.
        thread_id: String,
    },
}

/// Receives what the surfaces must do; the Mac app implements it in Swift.
///
/// `on_event` is called **on the hub thread**, inside the actor's turn,
/// in order, after the change is stored. So an implementation must:
///
/// - **Return at once.** Hop to the main actor (`Task { @MainActor in … }`)
///   and do the work there; events after this one wait for the call.
/// - **Never call back into the hub synchronously.** The actor is busy
///   delivering this event and cannot run another command: waiting on a
///   `GentboxHub` call from here (a semaphore, `DispatchQueue.main.sync` while
///   the main thread awaits the hub, a blocking wrapper) deadlocks the hub.
///   Call it from the main actor afterwards instead.
/// - **Not panic or throw.** A Rust implementation should also not
///   `tokio::spawn` here: it would land on the hub's own single-threaded
///   runtime and compete with every command.
#[uniffi::export(with_foreign)]
pub trait HubDelegate: Send + Sync {
    /// One event. Called on the hub thread; return at once (see the trait).
    fn on_event(&self, event: HubEvent);
}

/// Adapts the foreign delegate to the hub's delegate trait.
pub(crate) struct DelegateBridge(pub(crate) Arc<dyn HubDelegate>);

impl fmt::Debug for DelegateBridge {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DelegateBridge")
    }
}

impl gentbox_hub::HubDelegate for DelegateBridge {
    fn on_event(&self, event: CoreHubEvent) {
        self.0.on_event(HubEvent::from(event));
    }
}

impl From<CoreHubEvent> for HubEvent {
    fn from(event: CoreHubEvent) -> Self {
        match event {
            CoreHubEvent::Delivered {
                message,
                plan,
                offered_actions,
                has_live_waiter,
                replaces,
                pings,
            } => Self::Delivered {
                message: InboxMessage::new(&message, &offered_actions, has_live_waiter),
                plan: DeliveryPlan::from(&plan),
                replaces: replaces.map(|id| id.as_str().to_owned()),
                pings,
            },
            CoreHubEvent::Repinged { message_id, plan } => Self::Repinged {
                message_id: message_id.as_str().to_owned(),
                plan: DeliveryPlan::from(&plan),
            },
            CoreHubEvent::Repeated {
                message_id,
                repeat_count,
            } => Self::Repeated {
                message_id: message_id.as_str().to_owned(),
                repeat_count,
            },
            CoreHubEvent::Updated {
                message,
                offered_actions,
                has_live_waiter,
                take_down,
            } => Self::Updated {
                message: InboxMessage::new(&message, &offered_actions, has_live_waiter),
                take_down: take_down.into(),
            },
            CoreHubEvent::Closed {
                message_id,
                state,
                take_down,
            } => Self::Closed {
                message_id: message_id.as_str().to_owned(),
                state: state.into(),
                take_down: take_down.into(),
            },
            CoreHubEvent::Withdrawn(rescission) => Self::Withdrawn {
                message_id: rescission.message_id.as_str().to_owned(),
                reason: rescission.reason.into(),
            },
            CoreHubEvent::WaiterChanged {
                message_id,
                has_live_waiter,
                offered_actions,
            } => Self::WaiterChanged {
                message_id: message_id.as_str().to_owned(),
                has_live_waiter,
                offered_actions: offered_actions.iter().map(InboxAction::from).collect(),
            },
            CoreHubEvent::TopicChanged(topic_id) => Self::TopicChanged {
                topic_id: topic_id.as_str().to_owned(),
            },
            CoreHubEvent::ThreadChanged(thread_id) => Self::ThreadChanged {
                thread_id: thread_id.as_str().to_owned(),
            },
        }
    }
}
