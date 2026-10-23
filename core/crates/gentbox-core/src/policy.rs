//! The noise-policy engine (architecture §6, ADR 0004): a pure function from
//! a message, the context and the policy to a [`DeliveryPlan`].
//!
//! `type` picks the surface, `blocking` drives persistence and escalation,
//! `urgent` drives interruption. v1 has no phone channel: the plan's phone
//! part is always [`PhonePlan::None`].

use std::collections::BTreeMap;
use std::time::Duration;

use crate::message::{Blocking, Message, MessageType};

/// Whether you are at the Mac.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PresenceState {
    /// Unlocked with input within `awayAfter`.
    Present,
    /// Idle for at least `awayAfter`.
    Away,
    /// Screen locked.
    Locked,
}

/// What the platform side senses and pushes into the core.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Presence {
    /// Present, away or locked.
    pub state: PresenceState,
    /// The frontmost app's bundle id.
    pub frontmost_bundle_id: Option<String>,
}

/// The phone channel, reserved for the mobile release (M1). v1 never builds
/// one, and `decide` ignores it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PhoneContext {}

/// Everything about the moment that `decide` needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyContext {
    /// Presence and the frontmost app.
    pub presence: Presence,
    /// A Focus is on and Gentbox is not on its allowed list.
    pub focus_active: bool,
    /// Quiet hours are active.
    pub quiet_hours_active: bool,
    /// The message's topic or thread is muted (see
    /// [`crate::topic::is_effectively_muted`]).
    pub is_muted: bool,
    /// The phone channel; `None` in v1.
    pub phone: Option<PhoneContext>,
}

/// Where the Mac shows a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

/// How the native notification interrupts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NativeLevel {
    /// No native notification.
    None,
    /// An ordinary notification.
    Active,
    /// Time Sensitive: breaks through Focus.
    TimeSensitive,
}

/// The phone part of a plan. v1 has only `None`; the phone columns arrive
/// with M1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PhonePlan {
    /// No phone delivery and no escalation timer.
    None,
}

/// How often a still-open message pings again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reping {
    /// The gap between pings.
    pub after: Duration,
    /// At most this many re-pings.
    pub max: u8,
}

/// The blocking badge: independent of urgency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockingBadge {
    /// `non_blocking`: none.
    None,
    /// `blocking`: "Waiting on you".
    WaitingOnYou,
    /// `deferrable`: "Paused · can defer".
    PausedCanDefer,
}

/// Where a message sorts: urgent items above the rest, within a topic and
/// thread, in the inbox and in the popup stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SortTier {
    /// Urgent: above everything else.
    Urgent,
    /// Everything else, by time.
    Normal,
}

/// Why a plan looks the way it does: the "why this pinged" line and the
/// delivery log read these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanReason {
    /// The topic or thread is muted and the message is not urgent.
    Muted,
    /// Quiet hours held a non-urgent message back.
    QuietHours,
    /// Focus held a non-urgent message back.
    Focus,
    /// An urgent message broke through quiet hours or Focus.
    UrgentBreakthrough,
    /// A completion toast was suppressed: you are in the conversation's app.
    FrontmostSuppression,
}

/// What to do with a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryPlan {
    /// Where the Mac shows it.
    pub mac: MacSurface,
    /// The native notification.
    pub native_notification: NativeLevel,
    /// Whether it makes a sound (re-pings follow it).
    pub sound: bool,
    /// The phone part: always `None` in v1.
    pub phone: PhonePlan,
    /// Re-ping cadence while the message stays `open`.
    pub reping: Option<Reping>,
    /// Whether the Defer button is offered.
    pub offer_defer: bool,
    /// Whether a waiting sender should long-poll; `false` makes a hook fall
    /// back to the agent's own prompt at once.
    pub should_wait: bool,
    /// The red urgent badge under the type icon. Urgency never prefixes a
    /// title; native notifications lead the subtitle with "Urgent ·" instead.
    pub urgent_badge: bool,
    /// The blocking badge.
    pub blocking_badge: BlockingBadge,
    /// Where it sorts.
    pub sort_tier: SortTier,
    /// Why the plan is what it is.
    pub reasons: Vec<PlanReason>,
}

/// A cell of the policy table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PolicyKey {
    /// The type.
    pub message_type: MessageType,
    /// Whether urgent.
    pub urgent: bool,
    /// The blocking level.
    pub blocking: Blocking,
}

impl PolicyKey {
    /// The cell a message falls in.
    pub fn of(message: &Message) -> Self {
        Self {
            message_type: message.message_type,
            urgent: message.urgent,
            blocking: message.blocking,
        }
    }

    /// Whether the combination is one of the 15 valid ones.
    pub fn is_valid(&self) -> bool {
        (self.message_type.can_block() || self.blocking == Blocking::NonBlocking)
            && (self.message_type.can_be_urgent() || !self.urgent)
    }

    /// Every valid combination: 6 each for `decision_required` and
    /// `action_required`, 2 for `task_completed`, 1 for `generic`.
    pub fn all_valid() -> Vec<Self> {
        let mut keys = Vec::new();
        for message_type in MessageType::ALL {
            for urgent in [false, true] {
                for blocking in Blocking::ALL {
                    let key = Self {
                        message_type,
                        urgent,
                        blocking,
                    };
                    if key.is_valid() {
                        keys.push(key);
                    }
                }
            }
        }
        keys
    }
}

/// The configurable part of a cell, before the context modulates it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PolicyCell {
    /// The Mac surface.
    pub surface: MacSurface,
    /// Whether a native notification is posted.
    pub native_notification: bool,
    /// Whether it makes a sound.
    pub sound: bool,
    /// The re-ping cadence.
    pub reping: Option<Reping>,
    /// Whether Defer may be offered (it still needs a `deferrable` message).
    pub offer_defer: bool,
}

/// The policy table keyed on `(type, urgent, blocking)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    cells: BTreeMap<PolicyKey, PolicyCell>,
}

impl Default for Policy {
    /// The defaults of architecture §6.2.
    fn default() -> Self {
        let cells = PolicyKey::all_valid()
            .into_iter()
            .map(|key| (key, default_cell(&key)))
            .collect();
        Self { cells }
    }
}

impl Policy {
    /// The cell for a key; an invalid combination falls back to the cell of
    /// its normalized form.
    pub fn cell(&self, key: &PolicyKey) -> PolicyCell {
        let normalized = PolicyKey {
            message_type: key.message_type,
            urgent: key.urgent && key.message_type.can_be_urgent(),
            blocking: if key.message_type.can_block() {
                key.blocking
            } else {
                Blocking::NonBlocking
            },
        };
        self.cells
            .get(&normalized)
            .copied()
            .unwrap_or_else(|| default_cell(&normalized))
    }

    /// A copy of the policy with one cell replaced (what `config.toml` does).
    #[must_use]
    pub fn with_cell(mut self, key: PolicyKey, cell: PolicyCell) -> Self {
        self.cells.insert(key, cell);
        self
    }
}

fn default_cell(key: &PolicyKey) -> PolicyCell {
    match (key.message_type, key.blocking) {
        (MessageType::Generic, _) => PolicyCell {
            surface: MacSurface::InboxOnly,
            native_notification: false,
            sound: false,
            reping: None,
            offer_defer: false,
        },
        (MessageType::TaskCompleted, _) => PolicyCell {
            surface: MacSurface::Toast,
            native_notification: false,
            sound: true,
            reping: None,
            offer_defer: false,
        },
        (_, Blocking::Blocking) => persistent_cell(Duration::from_secs(5 * 60), 3, false),
        (_, Blocking::Deferrable) => persistent_cell(Duration::from_secs(10 * 60), 2, true),
        (_, Blocking::NonBlocking) => PolicyCell {
            surface: MacSurface::Banner,
            native_notification: true,
            sound: true,
            reping: None,
            offer_defer: false,
        },
    }
}

fn persistent_cell(after: Duration, max: u8, offer_defer: bool) -> PolicyCell {
    PolicyCell {
        surface: MacSurface::PersistentPopup,
        native_notification: true,
        sound: true,
        reping: Some(Reping { after, max }),
        offer_defer,
    }
}

/// Decides how loud a message is and where it appears.
///
/// - **Muted** (topic or thread via the context, or a `mute` rule recorded on
///   the message) and not urgent: inbox only, silent, no re-ping,
///   `should_wait: false`. Urgent messages are unaffected.
/// - **Quiet hours**, non-urgent: popups stay but silent; toasts and banners
///   become inbox-only. **Focus**, non-urgent: popups and banners silent,
///   toasts inbox-only. Urgent messages break through both.
/// - **Urgent** native notifications are Time Sensitive.
/// - **Frontmost suppression:** a `task_completed` toast is suppressed while
///   you are present in the conversation's app.
/// - Re-pings keep the cell's cadence; a re-ping follows the plan's `sound`.
/// - Presence does not change the Mac plan; it matters only to the phone
///   columns of the mobile release.
pub fn decide(message: &Message, context: &PolicyContext, policy: &Policy) -> DeliveryPlan {
    let cell = policy.cell(&PolicyKey::of(message));
    let mut surface = cell.surface;
    let mut native = cell.native_notification;
    let mut sound = cell.sound;
    let mut reping = cell.reping;
    let mut reasons = Vec::new();
    let held_back = !message.urgent;
    let is_muted = is_held_by_mute(message, context);

    if is_muted {
        surface = MacSurface::InboxOnly;
        native = false;
        sound = false;
        reping = None;
        reasons.push(PlanReason::Muted);
    } else {
        if context.quiet_hours_active && held_back {
            (surface, native, sound) = quiet(surface, native);
            reasons.push(PlanReason::QuietHours);
        }
        if context.focus_active && held_back {
            (surface, native, sound) = focused(surface, native);
            reasons.push(PlanReason::Focus);
        }
        if message.urgent && (context.quiet_hours_active || context.focus_active) {
            reasons.push(PlanReason::UrgentBreakthrough);
        }
    }

    if is_toast_suppressed(message, context, surface) {
        surface = MacSurface::InboxOnly;
        native = false;
        sound = false;
        reasons.push(PlanReason::FrontmostSuppression);
    }

    let should_wait = expects_live_waiter(message, context);
    DeliveryPlan {
        mac: surface,
        native_notification: native_level(native, message.urgent),
        sound,
        phone: PhonePlan::None,
        reping,
        offer_defer: cell.offer_defer && message.blocking == Blocking::Deferrable && should_wait,
        should_wait,
        urgent_badge: message.urgent,
        blocking_badge: match message.blocking {
            Blocking::NonBlocking => BlockingBadge::None,
            Blocking::Deferrable => BlockingBadge::PausedCanDefer,
            Blocking::Blocking => BlockingBadge::WaitingOnYou,
        },
        sort_tier: if message.urgent {
            SortTier::Urgent
        } else {
            SortTier::Normal
        },
        reasons,
    }
}

/// Whether a mute (the topic's or thread's via the context, or a `mute` rule
/// recorded on the message) holds the message back: only a non-urgent message
/// is held, urgent ones break through.
pub fn is_held_by_mute(message: &Message, context: &PolicyContext) -> bool {
    (context.is_muted || message.muted_by_rule.is_some()) && !message.urgent
}

/// Whether the sender of an incoming message will long-poll for it: it
/// declared `awaitsResponse`, the type can be answered, and no mute makes
/// `should_wait` false. This is the plan's `should_wait`, available before the
/// message is stored; the hub passes it to coalescing as
/// [`crate::coalesce::CoalesceInput::will_have_live_waiter`], so a muted message
/// never displaces a message that has a live waiter.
pub fn expects_live_waiter(message: &Message, context: &PolicyContext) -> bool {
    message.awaits_response
        && message.message_type.is_answerable()
        && !is_held_by_mute(message, context)
}

/// Quiet hours: the Mac is silent; toasts and banners are inbox-only.
fn quiet(surface: MacSurface, native: bool) -> (MacSurface, bool, bool) {
    match surface {
        MacSurface::Toast | MacSurface::Banner => (MacSurface::InboxOnly, false, false),
        other => (other, native, false),
    }
}

/// Focus: popups and banners are silent (macOS filters the native
/// notification itself); toasts are inbox-only.
fn focused(surface: MacSurface, native: bool) -> (MacSurface, bool, bool) {
    match surface {
        MacSurface::Toast => (MacSurface::InboxOnly, false, false),
        other => (other, native, false),
    }
}

fn native_level(is_posted: bool, is_urgent: bool) -> NativeLevel {
    match (is_posted, is_urgent) {
        (false, _) => NativeLevel::None,
        (true, false) => NativeLevel::Active,
        (true, true) => NativeLevel::TimeSensitive,
    }
}

fn is_toast_suppressed(message: &Message, context: &PolicyContext, surface: MacSurface) -> bool {
    if surface != MacSurface::Toast || context.presence.state != PresenceState::Present {
        return false;
    }
    let conversation_app = message
        .conversation
        .focus
        .as_ref()
        .and_then(|focus| focus.bundle_id.as_deref());
    match (
        conversation_app,
        context.presence.frontmost_bundle_id.as_deref(),
    ) {
        (Some(conversation_app), Some(frontmost)) => conversation_app == frontmost,
        _ => false,
    }
}
