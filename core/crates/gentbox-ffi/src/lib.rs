//! UniFFI bindings of the Gentbox hub for the Mac app (architecture §3, §4).
//!
//! On the Mac the app is the hub: [`GentboxHub::start`] takes the hub lock on
//! the state folder, opens the store, and serves the local API on the Unix
//! socket and loopback, exactly as `gentboxd` does, but with the app's UI as
//! the answer channel. The app talks to the core through this crate, never
//! over HTTP.
//!
//! | Module | Role |
//! | --- | --- |
//! | `hub` | [`GentboxHub`]: start, stop, the inbox query, answers, groups, presence |
//! | `events` | [`HubDelegate`], the callback interface, and [`HubEvent`] |
//! | `inbox` | the inbox grouped topic → thread → messages, with a Closed section |
//! | `records` | the value types Swift sees, converted from the core's |
//! | `error` | [`GentboxError`] |
//!
//! **Threading.** Every query and command is `async` on the Swift side and runs
//! on this crate's own Tokio runtime, never on the caller's thread. The
//! delegate is the one exception: it is called on the hub thread, so the
//! Swift side hops to the main actor and returns at once (see
//! [`HubDelegate`]).
//!
//! This is the only crate allowed `unsafe`, for UniFFI's generated scaffolding;
//! it has its own `[lints]` table.

mod error;
mod events;
mod hub;
mod inbox;
mod records;

pub use error::GentboxError;
pub use events::{HubDelegate, HubEvent};
pub use hub::{GentboxHub, HubOptions};
pub use inbox::{ClosedGroup, GroupKind, Inbox, InboxThread, InboxTopic, MutedGroup, TopicChoice};
pub use records::{
    ActionKind, Blocking, BlockingBadge, DeliveryPlan, InboxAction, InboxMessage, LifecycleState,
    MacSurface, MessageType, NativeLevel, PlanReason, PresenceState, PresenceUpdate, RescindReason,
    Surfaces,
};

uniffi::setup_scaffolding!();
