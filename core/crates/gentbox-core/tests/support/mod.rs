//! Fixtures shared by the table tests.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use gentbox_core::classify::{BuiltInRules, RuleSet};
use gentbox_core::ids::{MessageId, ThreadId, TopicId};
use gentbox_core::message::{Blocking, Message, MessageInput, MessageType};
use gentbox_core::policy::{PolicyContext, Presence, PresenceState};
use gentbox_core::prepare::{IngestSettings, prepare_message};
use serde_json::{Value, json};
use time::OffsetDateTime;
use time::macros::datetime;

pub const NOW: OffsetDateTime = datetime!(2026-10-24 12:00:00 UTC);

/// A minimal valid message with the given extra fields merged in.
pub fn input(extra: Value) -> MessageInput {
    let mut base = json!({
        "version": 1,
        "source": { "agent": "claude-code", "displayName": "Claude Code" },
        "conversation": { "id": "conversation-1", "cwd": "/Users/me/Developer/gentbox" },
        "title": "Ingest tests pass"
    });
    if let (Some(target), Some(extra)) = (base.as_object_mut(), extra.as_object()) {
        for (key, value) in extra {
            target.insert(key.clone(), value.clone());
        }
    }
    serde_json::from_value(base).unwrap()
}

/// Prepares a message with no user rules and default settings.
pub fn prepared_message_input(extra: Value) -> MessageInput {
    prepare_message(
        input(extra),
        &RuleSet::empty(),
        BuiltInRules::load(),
        IngestSettings::default(),
    )
    .unwrap()
    .message
}

/// A stored message with the given declared fields.
pub fn stored(
    message_type: MessageType,
    urgent: bool,
    blocking: Blocking,
    extra: Value,
) -> Message {
    let mut fields = json!({
        "type": message_type.as_str(),
        "urgent": urgent,
        "blocking": blocking.as_str(),
    });
    if let (Some(target), Some(extra)) = (fields.as_object_mut(), extra.as_object()) {
        for (key, value) in extra {
            target.insert(key.clone(), value.clone());
        }
    }
    let prepared = prepare_message(
        input(fields),
        &RuleSet::empty(),
        BuiltInRules::load(),
        IngestSettings::default(),
    )
    .unwrap();
    prepared.into_message(
        MessageId::new("01JBM3Z8Q7Y4K2N6P0R5T9VWXA"),
        NOW,
        TopicId::new("01JBM6R3K9P2W5X8Z1C4V7N0QD"),
        ThreadId::new("01JBM9P3H4S5E6J7N8G9E0S1TQ"),
    )
}

/// A stored message that awaits a response. A decision carries one button of
/// each answer kind, so every answer is one the message offers.
pub fn awaiting(message_type: MessageType, urgent: bool, blocking: Blocking) -> Message {
    let extra = if message_type == MessageType::DecisionRequired {
        json!({ "awaitsResponse": true, "actions": decision_buttons() })
    } else {
        json!({ "awaitsResponse": true })
    };
    stored(message_type, urgent, blocking, extra)
}

/// Approve, deny, a choice and a reply.
pub fn decision_buttons() -> Value {
    json!([
        { "id": "allow", "label": "Allow", "kind": "approve" },
        { "id": "refuse", "label": "Deny", "kind": "deny" },
        { "id": "pick-a", "label": "Option A", "kind": "choice", "value": "a" },
        { "id": "other", "label": "Other…", "kind": "reply" }
    ])
}

/// Every valid combination, as `(type, urgent, blocking)`.
pub fn valid_combinations() -> Vec<(MessageType, bool, Blocking)> {
    gentbox_core::policy::PolicyKey::all_valid()
        .into_iter()
        .map(|key| (key.message_type, key.urgent, key.blocking))
        .collect()
}

/// A calm context: present, no Focus, no quiet hours, not muted.
pub fn calm_context() -> PolicyContext {
    PolicyContext {
        presence: Presence {
            state: PresenceState::Present,
            frontmost_bundle_id: None,
        },
        focus_active: false,
        quiet_hours_active: false,
        is_muted: false,
        phone: None,
    }
}

/// A stored message with its own id and receive time.
pub fn stored_at(
    id: &str,
    received_at: OffsetDateTime,
    message_type: MessageType,
    urgent: bool,
    blocking: Blocking,
    extra: Value,
) -> Message {
    let mut message = stored(message_type, urgent, blocking, extra);
    message.id = MessageId::new(id);
    message.received_at = received_at;
    message
}

/// The snapshot of a stored message as coalescing sees it.
pub fn item_of(message: &Message, has_live_waiter: bool) -> gentbox_core::item::OpenItem {
    let plan = gentbox_core::policy::decide(
        message,
        &calm_context(),
        &gentbox_core::policy::Policy::default(),
    );
    gentbox_core::item::OpenItem::from_message(message, has_live_waiter, &plan)
}
