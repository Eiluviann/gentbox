//! The Claude Code mapping of architecture §5.3, row by row.
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;

use gentbox_core::classify::{BuiltInRules, FieldSource, RuleSet, classify};
use gentbox_core::message::{Blocking, MessageType};
use serde_json::json;
use support::input;

use Blocking::{Blocking as Blocks, NonBlocking};
use MessageType::{ActionRequired, DecisionRequired, Generic, TaskCompleted};

const ROWS: &[(&str, MessageType, Blocking)] = &[
    ("claude-code.permission_request", DecisionRequired, Blocks),
    (
        "claude-code.notification.permission_prompt",
        DecisionRequired,
        Blocks,
    ),
    (
        "claude-code.notification.elicitation_dialog",
        DecisionRequired,
        Blocks,
    ),
    (
        "claude-code.notification.elicitation_url_dialog",
        ActionRequired,
        Blocks,
    ),
    (
        "claude-code.notification.agent_needs_input",
        DecisionRequired,
        Blocks,
    ),
    (
        "claude-code.notification.quota_auto_resume_stale",
        ActionRequired,
        Blocks,
    ),
    ("claude-code.notification.idle_prompt", Generic, NonBlocking),
    (
        "claude-code.notification.agent_completed",
        TaskCompleted,
        NonBlocking,
    ),
    (
        "claude-code.notification.auth_success",
        Generic,
        NonBlocking,
    ),
    (
        "claude-code.notification.elicitation_complete",
        Generic,
        NonBlocking,
    ),
    (
        "claude-code.notification.elicitation_response",
        Generic,
        NonBlocking,
    ),
    (
        "claude-code.notification.quota_auto_resume_fired",
        Generic,
        NonBlocking,
    ),
    (
        "claude-code.notification.quota_auto_resume_disabled",
        Generic,
        NonBlocking,
    ),
    (
        "claude-code.notification.plugin_notification",
        Generic,
        NonBlocking,
    ),
    ("claude-code.stop", TaskCompleted, NonBlocking),
    ("claude-code.stop.paused", Generic, NonBlocking),
    ("claude-code.stop_failure", ActionRequired, Blocks),
    ("claude-code.subagent_stop", Generic, NonBlocking),
];

#[test]
fn the_embedded_rules_parse_and_cover_every_row_once() {
    let built_in = BuiltInRules::load();
    // 13 rules: the six informational notification types share one rule.
    assert_eq!(built_in.rules().len(), 13);
    assert_eq!(ROWS.len(), 18);
}

#[test]
fn every_claude_code_event_maps_to_its_row() {
    for &(event, expected_type, expected_blocking) in ROWS {
        let classification = classify(
            &input(json!({ "event": event })),
            &RuleSet::empty(),
            BuiltInRules::load(),
        );
        assert_eq!(classification.message_type.value, expected_type, "{event}");
        assert_eq!(classification.blocking.value, expected_blocking, "{event}");
        assert!(
            !classification.urgent.value,
            "{event}: urgent is false unless a user rule says so"
        );
        assert!(
            matches!(
                classification.message_type.source,
                FieldSource::BuiltInRule(_)
            ),
            "{event}"
        );
    }
}

#[test]
fn near_miss_events_and_other_senders_get_defaults() {
    for event in [
        "claude-code.notification.unknown",
        "claude-code.stopped",
        "claude-code.stop.paused.extra",
        "codex.stop",
        "stop",
    ] {
        let classification = classify(
            &input(json!({ "event": event })),
            &RuleSet::empty(),
            BuiltInRules::load(),
        );
        assert_eq!(
            classification.message_type.source,
            FieldSource::Default,
            "{event}"
        );
        assert_eq!(classification.message_type.value, Generic, "{event}");
    }
}

#[test]
fn a_hook_that_declares_fields_keeps_them() {
    let classification = classify(
        &input(
            json!({ "event": "claude-code.permission_request", "blocking": "deferrable", "urgent": true }),
        ),
        &RuleSet::empty(),
        BuiltInRules::load(),
    );
    assert_eq!(classification.blocking.value, Blocking::Deferrable);
    assert!(classification.urgent.value);
    assert_eq!(classification.message_type.value, DecisionRequired);
}

#[test]
fn a_user_rule_makes_work_permission_prompts_urgent() {
    let rules = RuleSet::parse_with_home(
        r#"
        [[rule]]
        name = "Permission prompts at work are urgent"
        match = { source = "claude-code", event = "claude-code.permission_request", cwd = "~/Work/**" }
        urgent = true
        override = true
        "#,
        "/Users/me",
    )
    .unwrap();
    let message = input(json!({
        "event": "claude-code.permission_request",
        "conversation": { "id": "c", "cwd": "/Users/me/Work/api" }
    }));
    let classification = classify(&message, &rules, BuiltInRules::load());
    assert!(classification.urgent.value);
    assert_eq!(classification.blocking.value, Blocks);
}
