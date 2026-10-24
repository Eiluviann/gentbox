//! The value types Swift sees. Each mirrors a core type and is converted from
//! it here, so the core stays free of UniFFI and the Swift side never sees a
//! field it should not draw (bodies stay out of logs, `tool_input` never
//! reaches a title: the hub guarded both before storing).

use std::time::SystemTime;

use gentbox_core::message as core_message;
use gentbox_core::{lifecycle, policy, rescind};
use gentbox_hub::InboxEntry;

/// The four message types (ADR 0007).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum MessageType {
    /// The agent needs your answer.
    DecisionRequired,
    /// You must do something yourself, then say Done.
    ActionRequired,
    /// Work finished.
    TaskCompleted,
    /// Everything else, kept for reference.
    Generic,
}

/// How much the agent waits on you.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum Blocking {
    /// The agent keeps working.
    NonBlocking,
    /// The agent has paused; you may Defer.
    Deferrable,
    /// Work cannot continue until you decide.
    Blocking,
}

/// Where a message is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum LifecycleState {
    /// Needs attention.
    Open,
    /// Seen, still answerable.
    Acknowledged,
    /// Answered.
    Answered,
    /// Cleared without an answer.
    Dismissed,
    /// Withdrawn.
    Rescinded,
    /// Expired.
    Expired,
}

/// What a button does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum ActionKind {
    /// Approve.
    Approve,
    /// Deny.
    Deny,
    /// One of several choices.
    Choice,
    /// A text reply.
    Reply,
    /// You did what an `action_required` message asked.
    Done,
    /// Let the agent continue without your input for now.
    Defer,
    /// Open the conversation.
    Open,
    /// Remind me later; never reaches the agent.
    Snooze,
}

/// Why a message was withdrawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum RescindReason {
    /// Handled in the session.
    ResolvedElsewhere,
    /// No longer needed.
    Obsolete,
    /// Replaced by a newer message.
    Superseded,
}

/// Where the Mac shows a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum MacSurface {
    /// A persistent rich popup until handled.
    PersistentPopup,
    /// A transient rich toast.
    Toast,
    /// A native banner and the inbox.
    Banner,
    /// The inbox only.
    InboxOnly,
}

/// How a native notification interrupts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum NativeLevel {
    /// None.
    None,
    /// An ordinary notification.
    Active,
    /// Time Sensitive.
    TimeSensitive,
}

/// The blocking badge, independent of urgency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum BlockingBadge {
    /// No badge.
    None,
    /// "Agent waiting".
    WaitingOnYou,
    /// "Paused · can defer".
    PausedCanDefer,
}

/// Why a plan looks the way it does: the "why this pinged" line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum PlanReason {
    /// The topic or thread is muted.
    Muted,
    /// Quiet hours held it back.
    QuietHours,
    /// Focus held it back.
    Focus,
    /// Urgent: it broke through quiet hours or Focus.
    UrgentBreakthrough,
    /// You are in the conversation's app.
    FrontmostSuppression,
}

/// Whether you are at the Mac, as the platform side senses it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum PresenceState {
    /// Unlocked, with input within `awayAfter`.
    Present,
    /// Idle for at least `awayAfter`.
    Away,
    /// Screen locked.
    Locked,
}

/// What the presence monitor pushes into the core.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PresenceUpdate {
    /// Present, away or locked.
    pub state: PresenceState,
    /// The frontmost app's bundle id.
    pub frontmost_bundle_id: Option<String>,
}

/// One button.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct InboxAction {
    /// The id to send back with [`crate::GentboxHub::take_action`].
    pub id: String,
    /// The label.
    pub label: String,
    /// What it does.
    pub kind: ActionKind,
    /// The value a choice carries.
    pub value: Option<String>,
    /// The placeholder of a reply field.
    pub placeholder: Option<String>,
    /// Drawn in the system red.
    pub is_destructive: bool,
}

/// Which surfaces to take down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, uniffi::Record)]
pub struct Surfaces {
    /// The persistent popup.
    pub popup: bool,
    /// The toast.
    pub toast: bool,
    /// The native notification.
    pub native_notification: bool,
}

/// Where and how loud a message is delivered.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DeliveryPlan {
    /// The Mac surface.
    pub mac: MacSurface,
    /// The native notification.
    pub native_notification: NativeLevel,
    /// Whether it makes a sound.
    pub sound: bool,
    /// Whether Defer is offered.
    pub offer_defer: bool,
    /// The red urgent badge under the type icon.
    pub urgent_badge: bool,
    /// The blocking badge.
    pub blocking_badge: BlockingBadge,
    /// Why.
    pub reasons: Vec<PlanReason>,
}

/// One message row, with its conversation as metadata.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct InboxMessage {
    /// The hub id.
    pub id: String,
    /// The descriptive title (ADR 0010); shown as it is.
    pub title: String,
    /// The summary.
    pub summary: Option<String>,
    /// Markdown body: commands, paths and raw input live here only.
    pub body: Option<String>,
    /// The type.
    pub message_type: MessageType,
    /// Urgent: the red badge and sort to the top; never a title prefix.
    pub is_urgent: bool,
    /// The blocking level.
    pub blocking: Blocking,
    /// Open or acknowledged, for rows in the inbox.
    pub state: LifecycleState,
    /// `source.agent`.
    pub agent: String,
    /// The agent's display name, else its id.
    pub agent_name: String,
    /// The conversation's id.
    pub conversation_id: String,
    /// The conversation's title, else its project folder's name.
    pub conversation_title: Option<String>,
    /// The topic.
    pub topic_id: String,
    /// The thread.
    pub thread_id: String,
    /// When the hub received it.
    pub received_at: SystemTime,
    /// Whether a sender is waiting for the answer now.
    pub has_live_waiter: bool,
    /// The buttons to show now.
    pub offered_actions: Vec<InboxAction>,
    /// It asks for permission (it has an Approve button), whether or not a
    /// sender waits now: the `checkmark.shield` icon.
    pub asks_permission: bool,
    /// How many repeats it absorbed.
    pub repeat_count: u32,
    /// The steps of an action plan.
    pub step_count: u32,
    /// Its topic and thread, for the breadcrumb and the item menu; set in
    /// [`crate::GentboxHub::inbox`], `None` in events.
    pub group: Option<GroupContext>,
}

/// A message's topic and thread as a row shows them.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct GroupContext {
    /// The topic's title.
    pub topic_title: String,
    /// An agent's implicit General topic, which the breadcrumb leaves out.
    pub topic_is_general: bool,
    /// The topic is muted (and so its threads).
    pub topic_is_muted: bool,
    /// The thread's title; a conversation's default thread carries the
    /// conversation's.
    pub thread_title: String,
    /// The thread itself is muted.
    pub thread_is_muted: bool,
}

impl InboxMessage {
    pub(crate) fn from_entry(entry: &InboxEntry) -> Self {
        Self::new(
            &entry.message,
            &entry.offered_actions,
            entry.has_live_waiter,
        )
    }

    pub(crate) fn new(
        message: &core_message::Message,
        offered_actions: &[core_message::Action],
        has_live_waiter: bool,
    ) -> Self {
        let conversation_title = message
            .conversation
            .title
            .clone()
            .or_else(|| message.conversation.project_name().map(str::to_owned));
        Self {
            id: message.id.as_str().to_owned(),
            title: message.title.clone(),
            summary: message.summary.clone(),
            body: message.body.clone(),
            message_type: message.message_type.into(),
            is_urgent: message.urgent,
            blocking: message.blocking.into(),
            state: message.lifecycle.state.into(),
            agent: message.source.agent.clone(),
            agent_name: message
                .source
                .display_name
                .clone()
                .unwrap_or_else(|| message.source.agent.clone()),
            conversation_id: message.conversation.id.clone(),
            conversation_title,
            topic_id: message.topic_id.as_str().to_owned(),
            thread_id: message.thread_id.as_str().to_owned(),
            received_at: SystemTime::from(message.received_at),
            has_live_waiter,
            offered_actions: offered_actions.iter().map(InboxAction::from).collect(),
            asks_permission: message
                .actions
                .iter()
                .any(|action| action.kind == core_message::ActionKind::Approve),
            repeat_count: message.repeat_count,
            step_count: u32::try_from(message.steps.len()).unwrap_or(u32::MAX),
            group: None,
        }
    }

    /// The agent waits on you: blocking or deferrable.
    pub(crate) fn is_waiting(&self) -> bool {
        self.blocking != Blocking::NonBlocking
    }
}

impl From<core_message::MessageType> for MessageType {
    fn from(value: core_message::MessageType) -> Self {
        match value {
            core_message::MessageType::DecisionRequired => Self::DecisionRequired,
            core_message::MessageType::ActionRequired => Self::ActionRequired,
            core_message::MessageType::TaskCompleted => Self::TaskCompleted,
            core_message::MessageType::Generic => Self::Generic,
        }
    }
}

impl From<core_message::Blocking> for Blocking {
    fn from(value: core_message::Blocking) -> Self {
        match value {
            core_message::Blocking::NonBlocking => Self::NonBlocking,
            core_message::Blocking::Deferrable => Self::Deferrable,
            core_message::Blocking::Blocking => Self::Blocking,
        }
    }
}

impl From<lifecycle::LifecycleState> for LifecycleState {
    fn from(value: lifecycle::LifecycleState) -> Self {
        match value {
            lifecycle::LifecycleState::Open => Self::Open,
            lifecycle::LifecycleState::Acknowledged => Self::Acknowledged,
            lifecycle::LifecycleState::Answered => Self::Answered,
            lifecycle::LifecycleState::Dismissed => Self::Dismissed,
            lifecycle::LifecycleState::Rescinded => Self::Rescinded,
            lifecycle::LifecycleState::Expired => Self::Expired,
        }
    }
}

impl From<core_message::ActionKind> for ActionKind {
    fn from(value: core_message::ActionKind) -> Self {
        match value {
            core_message::ActionKind::Approve => Self::Approve,
            core_message::ActionKind::Deny => Self::Deny,
            core_message::ActionKind::Choice => Self::Choice,
            core_message::ActionKind::Reply => Self::Reply,
            core_message::ActionKind::Done => Self::Done,
            core_message::ActionKind::Defer => Self::Defer,
            core_message::ActionKind::Open => Self::Open,
            core_message::ActionKind::Snooze => Self::Snooze,
        }
    }
}

impl From<&core_message::Action> for InboxAction {
    fn from(action: &core_message::Action) -> Self {
        Self {
            id: action.id.clone(),
            label: action.label.clone(),
            kind: action.kind.into(),
            value: action.value.clone(),
            placeholder: action.placeholder.clone(),
            is_destructive: action.destructive,
        }
    }
}

impl From<rescind::RescindReason> for RescindReason {
    fn from(value: rescind::RescindReason) -> Self {
        match value {
            rescind::RescindReason::ResolvedElsewhere => Self::ResolvedElsewhere,
            rescind::RescindReason::Obsolete => Self::Obsolete,
            rescind::RescindReason::Superseded => Self::Superseded,
        }
    }
}

impl From<gentbox_core::item::DeliveredSurfaces> for Surfaces {
    fn from(value: gentbox_core::item::DeliveredSurfaces) -> Self {
        Self {
            popup: value.popup,
            toast: value.toast,
            native_notification: value.native_notification,
        }
    }
}

impl From<&policy::DeliveryPlan> for DeliveryPlan {
    fn from(plan: &policy::DeliveryPlan) -> Self {
        Self {
            mac: match plan.mac {
                policy::MacSurface::PersistentPopup => MacSurface::PersistentPopup,
                policy::MacSurface::Toast => MacSurface::Toast,
                policy::MacSurface::Banner => MacSurface::Banner,
                policy::MacSurface::InboxOnly => MacSurface::InboxOnly,
            },
            native_notification: match plan.native_notification {
                policy::NativeLevel::None => NativeLevel::None,
                policy::NativeLevel::Active => NativeLevel::Active,
                policy::NativeLevel::TimeSensitive => NativeLevel::TimeSensitive,
            },
            sound: plan.sound,
            offer_defer: plan.offer_defer,
            urgent_badge: plan.urgent_badge,
            blocking_badge: match plan.blocking_badge {
                policy::BlockingBadge::None => BlockingBadge::None,
                policy::BlockingBadge::WaitingOnYou => BlockingBadge::WaitingOnYou,
                policy::BlockingBadge::PausedCanDefer => BlockingBadge::PausedCanDefer,
            },
            reasons: plan
                .reasons
                .iter()
                .map(|reason| match reason {
                    policy::PlanReason::Muted => PlanReason::Muted,
                    policy::PlanReason::QuietHours => PlanReason::QuietHours,
                    policy::PlanReason::Focus => PlanReason::Focus,
                    policy::PlanReason::UrgentBreakthrough => PlanReason::UrgentBreakthrough,
                    policy::PlanReason::FrontmostSuppression => PlanReason::FrontmostSuppression,
                })
                .collect(),
        }
    }
}

impl From<PresenceUpdate> for policy::Presence {
    fn from(update: PresenceUpdate) -> Self {
        Self {
            state: match update.state {
                PresenceState::Present => policy::PresenceState::Present,
                PresenceState::Away => policy::PresenceState::Away,
                PresenceState::Locked => policy::PresenceState::Locked,
            },
            frontmost_bundle_id: update.frontmost_bundle_id,
        }
    }
}
