//! Gentbox persistence: a synchronous SQLite store for the types of
//! `gentbox-core`, its migrations and the retention sweep.
//!
//! - [`Store`] opens the file (WAL, foreign keys on, migrated) and groups
//!   writes into transactions.
//! - [`Repository`] is the API: messages, conversations, actions, answers and
//!   the delivery log; topics, threads and default threads; the sweep.
//! - There is no clock here: callers pass `now`. There is no async: the hub
//!   actor in PR 2b owns a [`Store`] and is its only writer.
//! - Not-found and constraint outcomes are [`StoreError`] values. A caller
//!   that breaks a documented precondition gets a panic.
//!
//! See `docs/architecture.md` §5.2 (lifecycle, retention) and ADRs 0008 and
//! 0009 (topics and threads).

mod codec;
mod error;
mod groups;
mod migrations;
mod repository;
mod retention;
mod store;

pub use error::{Conflict, EntityKind, StoreError};
pub use groups::{ThreadReopened, Upserted};
pub use migrations::latest_version;
pub use repository::{DeliveryEvent, DeliveryRecord, MessageFilter, Repository};
pub use retention::{DEFAULT_RETENTION, RetentionPolicy, SweepReport};
pub use store::Store;
