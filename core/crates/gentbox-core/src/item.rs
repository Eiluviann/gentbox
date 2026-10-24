//! A snapshot of a stored message, as coalescing and rescinding see it: the
//! facts they need, plus the live-waiter flag only the hub knows.

use time::OffsetDateTime;

use crate::ids::{ConversationKey, MessageId, ThreadId};
use crate::lifecycle::LifecycleState;
use crate::message::{Blocking, Message, MessageType};
use crate::policy::{DeliveryPlan, MacSurface};

/// Which Mac surfaces currently show a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DeliveredSurfaces {
    /// A persistent popup is on screen.
    pub popup: bool,
    /// A transient toast is on screen.
    pub toast: bool,
    /// A native notification was posted.
    pub native_notification: bool,
}

impl DeliveredSurfaces {
    /// The surfaces a delivery plan put the message on.
    pub fn from_plan(plan: &DeliveryPlan) -> Self {
        Self {
            popup: plan.mac == MacSurface::PersistentPopup,
            toast: plan.mac == MacSurface::Toast,
            native_notification: plan.native_notification != crate::policy::NativeLevel::None,
        }
    }
}

/// What coalescing and rescinding need to know about a stored message.
///
/// The hub builds one per open or acknowledged message of the conversation in
/// question; items of other conversations are ignored by every function that
/// takes a slice of them, because topics and threads never coalesce.
#[derive(Debug, Clone, PartialEq)]
pub struct OpenItem {
    /// The message id.
    pub id: MessageId,
    /// The conversation it belongs to.
    pub conversation: ConversationKey,
    /// The thread it is in (informational: grouping never affects coalescing).
    pub thread_id: ThreadId,
    /// Its type.
    pub message_type: MessageType,
    /// Whether it is urgent.
    pub urgent: bool,
    /// Its blocking level.
    pub blocking: Blocking,
    /// Its title.
    pub title: String,
    /// Its summary.
    pub summary: Option<String>,
    /// The slot it occupies.
    pub correlation_key: Option<String>,
    /// Open or acknowledged.
    pub state: LifecycleState,
    /// A long-poll for it is in flight or ended under 10 s ago.
    pub has_live_waiter: bool,
    /// When the hub received it.
    pub received_at: OffsetDateTime,
    /// Where it is currently shown.
    pub delivered: DeliveredSurfaces,
    /// The hub parked it: completion and progress leave it open.
    pub is_parked: bool,
}

impl OpenItem {
    /// Snapshots a stored message together with the hub's live-waiter flag
    /// and the plan it was delivered with.
    pub fn from_message(message: &Message, has_live_waiter: bool, plan: &DeliveryPlan) -> Self {
        Self {
            id: message.id.clone(),
            conversation: message.conversation_key(),
            thread_id: message.thread_id.clone(),
            message_type: message.message_type,
            urgent: message.urgent,
            blocking: message.blocking,
            title: message.title.clone(),
            summary: message.summary.clone(),
            correlation_key: message.correlation_key.clone(),
            state: message.lifecycle.state,
            has_live_waiter,
            received_at: message.received_at,
            delivered: DeliveredSurfaces::from_plan(plan),
            is_parked: message.is_parked(),
        }
    }

    /// Open or acknowledged.
    pub fn is_live(&self) -> bool {
        self.state.is_live()
    }

    /// `blocking` or `deferrable`.
    pub fn is_waiting_on_you(&self) -> bool {
        self.blocking.is_waiting()
    }
}
