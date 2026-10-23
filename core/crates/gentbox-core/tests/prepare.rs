//! The pure ingest pipeline end to end: validate → classify → guard → plan.
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;

use gentbox_core::classify::{BuiltInRules, FieldSource, RuleSet};
use gentbox_core::ids::{MessageId, ThreadId, TopicId};
use gentbox_core::lifecycle::LifecycleState;
use gentbox_core::message::{Blocking, Message, MessageType};
use gentbox_core::prepare::{IngestRejection, IngestSettings, prepare_message};
use gentbox_core::response::{IngestResponse, WarningCode};
use serde_json::json;
use support::{NOW, input, stored};

fn prepare(
    extra: serde_json::Value,
) -> Result<gentbox_core::prepare::PreparedMessage, IngestRejection> {
    prepare_message(
        input(extra),
        &RuleSet::empty(),
        BuiltInRules::load(),
        IngestSettings::default(),
    )
}

#[test]
fn a_command_title_is_rewritten_with_a_warning_and_the_original_is_first_in_the_body() {
    let prepared = prepare(json!({
        "type": "decision_required", "blocking": "blocking",
        "title": "Allow Bash: cargo test -p gentbox-ingest -- --nocapture?",
        "summary": "Run the ingest tests with output",
        "body": "```bash\ncargo test -p gentbox-ingest -- --nocapture\n```",
    }))
    .unwrap();
    assert_eq!(prepared.message.title, "Run the ingest tests with output");
    assert!(
        prepared
            .message
            .body
            .unwrap()
            .starts_with("**Original title**")
    );
    let codes: Vec<_> = prepared
        .warnings
        .iter()
        .map(|warning| warning.code)
        .collect();
    assert_eq!(codes, vec![WarningCode::TitleLooksLikeCommand]);
}

#[test]
fn a_flagged_conversation_title_is_ignored_and_warned() {
    let prepared =
        prepare(json!({ "conversation": { "id": "c", "title": "cargo test --all" } })).unwrap();
    assert_eq!(prepared.message.conversation.title, None);
    assert!(
        prepared
            .warnings
            .iter()
            .any(|w| w.code == WarningCode::ConversationTitleIgnored)
    );
}

#[test]
fn normalizations_become_warnings_and_the_fields_are_final() {
    let rules = RuleSet::parse("[[rule]]\nname = \"Quiet\"\ntype = \"generic\"\n").unwrap();
    let prepared = prepare_message(
        input(json!({ "urgent": true, "blocking": "blocking", "scope": "goal" })),
        &rules,
        BuiltInRules::load(),
        IngestSettings::default(),
    )
    .unwrap();
    assert_eq!(prepared.message.message_type, Some(MessageType::Generic));
    assert_eq!(prepared.message.urgent, Some(false));
    assert_eq!(prepared.message.blocking, Some(Blocking::NonBlocking));
    assert_eq!(prepared.message.scope, None);
    let normalized = prepared
        .warnings
        .iter()
        .filter(|w| w.code == WarningCode::Normalized)
        .count();
    assert_eq!(normalized, 3);
}

#[test]
fn declared_invalid_combinations_never_reach_classification() {
    let rejected = prepare(json!({ "type": "generic", "urgent": true }));
    match rejected {
        Err(IngestRejection::Invalid(error)) => assert_eq!(error.code(), "invalid_combination"),
        other => panic!("expected a 422, got {other:?}"),
    }
}

#[test]
fn the_stored_message_defaults_expiry_state_and_counters() {
    let message = stored(
        MessageType::DecisionRequired,
        false,
        Blocking::Blocking,
        json!({}),
    );
    assert_eq!(message.lifecycle.state, LifecycleState::Open);
    assert_eq!(message.expires_at, NOW + time::Duration::hours(24));
    assert_eq!(message.repeat_count, 0);
    assert!(message.steps_completed.is_empty());
    assert_eq!(message.field_sources.message_type, FieldSource::Declared);

    let generic = stored(
        MessageType::Generic,
        false,
        Blocking::NonBlocking,
        json!({}),
    );
    assert_eq!(
        generic.lifecycle.state,
        LifecycleState::Acknowledged,
        "generic never badges"
    );
}

#[test]
fn the_stored_message_serializes_camel_case_and_round_trips() {
    let message = stored(
        MessageType::ActionRequired,
        true,
        Blocking::Deferrable,
        json!({ "awaitsResponse": true }),
    );
    let json = serde_json::to_value(&message).unwrap();
    for key in [
        "topicId",
        "threadId",
        "fieldSources",
        "awaitsResponse",
        "receivedAt",
        "expiresAt",
        "repeatCount",
    ] {
        assert!(json.get(key).is_some(), "missing {key}");
    }
    assert_eq!(json["type"], "action_required");
    assert_eq!(json["fieldSources"]["type"], "declared");
    assert_eq!(json["lifecycle"]["state"], "open");
    let back: Message = serde_json::from_value(json).unwrap();
    assert_eq!(back, message);
}

#[test]
fn the_ingest_response_assembles_from_the_prepared_message() {
    let prepared = prepare(
        json!({ "type": "decision_required", "blocking": "blocking", "awaitsResponse": true }),
    )
    .unwrap();
    let warnings = prepared.warnings.clone();
    let field_sources = prepared.classification.field_sources();
    let message = prepared.into_message(
        MessageId::new("01JBMA2W8R6T4Y2V0P8N6M4K2H"),
        NOW,
        TopicId::new("01JBM6R3K9P2W5X8Z1C4V7N0QD"),
        ThreadId::new("01JBM9P3H4S5E6J7N8G9E0S1TQ"),
    );
    let response = IngestResponse {
        id: message.id.clone(),
        message_type: message.message_type,
        urgent: message.urgent,
        blocking: message.blocking,
        field_sources: Some(field_sources),
        topic_id: message.topic_id.clone(),
        thread_id: message.thread_id.clone(),
        state: message.lifecycle.state,
        should_wait: true,
        warnings,
    };
    let json = serde_json::to_value(&response).unwrap();
    assert_eq!(json["fieldSources"]["urgent"], "default");
    assert_eq!(json["state"], "open");
    assert!(json.get("warnings").is_none(), "no warnings, no key");
}

#[test]
fn metadata_is_kept_as_flat_sender_facts() {
    let message = stored(
        MessageType::Generic,
        false,
        Blocking::NonBlocking,
        json!({ "metadata": { "toolName": "Bash", "exitCode": 0, "isRetry": false } }),
    );
    assert_eq!(message.metadata.len(), 3);
}
