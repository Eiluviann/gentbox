//! Gentbox core: the message model, classification, the noise-policy engine,
//! topics and threads, the title guard and action plans, shared by the
//! menu-bar app, `gentboxd` and the FFI bindings.
//!
//! The crate is pure: no IO, no async, no clock (time is passed in) and no
//! `unsafe`. See `docs/architecture.md` §1, §5, §6 and §7 and ADRs 0007 to
//! 0011.
//!
//! The ingest pipeline of architecture §5.1 maps onto the modules like this:
//!
//! | Step | Module |
//! | --- | --- |
//! | validate, classify, secret scan, title guard, action plan | [`prepare`] on [`validate`], [`classify`], [`secret_scan`], [`title_guard`], [`action_plan`] |
//! | resolve topic and thread | [`resolve`] |
//! | coalesce, dedupe, supersede, rescind | [`coalesce`], [`rescind`] |
//! | policy engine | [`policy`] |
//! | states and buttons | [`lifecycle`], [`actions`] |

pub mod action_plan;
pub mod actions;
pub mod classify;
pub mod coalesce;
pub mod ids;
pub mod item;
pub mod lifecycle;
pub mod message;
pub mod policy;
pub mod prepare;
pub mod presentation;
pub mod rescind;
pub mod resolve;
pub mod response;
pub mod secret_scan;
pub mod template;
pub mod title_guard;
pub mod topic;
pub mod validate;

/// Version of the ingest message schema this core accepts
/// (`docs/schema/message-v1.json`).
pub const MESSAGE_SCHEMA_VERSION: u32 = 1;
