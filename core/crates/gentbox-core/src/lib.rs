//! Gentbox core: the message model, classification and the
//! noise-policy engine shared by the menu-bar app, `gentboxd` and the
//! FFI bindings. See `docs/architecture.md`; the implementation lands in
//! Phase 3 PR 1 (`docs/implementation-plan.md`).

/// Version of the ingest message schema this core accepts
/// (`docs/schema/message-v1.json`).
pub const MESSAGE_SCHEMA_VERSION: u32 = 1;
