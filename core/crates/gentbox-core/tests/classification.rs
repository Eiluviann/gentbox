//! Classification and validation tables (architecture §5.3, ADR 0007).
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;

use gentbox_core::classify::{
    BuiltInRules, FieldSource, NormalizedField, RuleParseError, RuleSet, classify,
};
use gentbox_core::message::{Blocking, MessageScope, MessageType};
use gentbox_core::validate::{CombinationViolation, ValidationError, validate_message};
use serde_json::json;
use support::{input, valid_combinations};

fn classify_with(rules: &str, extra: serde_json::Value) -> gentbox_core::classify::Classification {
    classify(
        &input(extra),
        &RuleSet::parse(rules).unwrap(),
        BuiltInRules::load(),
    )
}

fn classify_plain(extra: serde_json::Value) -> gentbox_core::classify::Classification {
    classify(&input(extra), &RuleSet::empty(), BuiltInRules::load())
}

#[test]
fn all_fifteen_valid_combinations_are_accepted_when_declared() {
    let combinations = valid_combinations();
    assert_eq!(combinations.len(), 15);
    for (message_type, urgent, blocking) in combinations {
        let fields = json!({
            "type": message_type.as_str(), "urgent": urgent, "blocking": blocking.as_str()
        });
        assert!(
            validate_message(&input(fields.clone())).is_ok(),
            "{message_type:?} {urgent} {blocking:?}"
        );
        let classification = classify_plain(fields);
        assert_eq!(classification.message_type.value, message_type);
        assert_eq!(classification.urgent.value, urgent);
        assert_eq!(classification.blocking.value, blocking);
        assert!(classification.normalizations.is_empty());
    }
}

#[test]
fn every_invalid_declared_combination_is_rejected() {
    let mut rejected = 0;
    for message_type in MessageType::ALL {
        for urgent in [false, true] {
            for blocking in Blocking::ALL {
                let is_valid = (message_type.can_block() || blocking == Blocking::NonBlocking)
                    && (message_type.can_be_urgent() || !urgent);
                let message = input(json!({
                    "type": message_type.as_str(), "urgent": urgent, "blocking": blocking.as_str()
                }));
                let result = validate_message(&message);
                assert_eq!(
                    result.is_ok(),
                    is_valid,
                    "{message_type:?} {urgent} {blocking:?}"
                );
                if let Err(error) = result {
                    assert_eq!(error.code(), "invalid_combination");
                    rejected += 1;
                }
            }
        }
    }
    assert_eq!(
        rejected,
        24 - 15,
        "4 types x 2 x 3 = 24 combinations, 15 valid"
    );
}

#[test]
fn scope_and_steps_on_the_wrong_declared_type_are_rejected() {
    let scope = input(json!({ "type": "generic", "scope": "goal" }));
    assert_eq!(
        validate_message(&scope),
        Err(ValidationError::InvalidCombination(
            CombinationViolation::ScopeOnWrongType(MessageType::Generic)
        ))
    );
    let steps = input(json!({ "type": "decision_required", "steps": [{ "text": "Sign in" }] }));
    assert_eq!(
        validate_message(&steps),
        Err(ValidationError::InvalidCombination(
            CombinationViolation::StepsOnWrongType(MessageType::DecisionRequired)
        ))
    );
    assert!(validate_message(&input(json!({ "type": "task_completed", "scope": "goal" }))).is_ok());
}

#[test]
fn rule_filled_invalid_combinations_are_normalized_and_recorded() {
    let rules = r#"
        [[rule]]
        name = "Everything generic"
        type = "generic"
    "#;
    let classification = classify_with(rules, json!({ "urgent": true, "blocking": "blocking" }));
    assert_eq!(classification.message_type.value, MessageType::Generic);
    assert!(!classification.urgent.value);
    assert_eq!(classification.urgent.source, FieldSource::Normalized);
    assert_eq!(classification.blocking.value, Blocking::NonBlocking);
    assert_eq!(classification.blocking.source, FieldSource::Normalized);
    let fields: Vec<_> = classification
        .normalizations
        .iter()
        .map(|normalization| normalization.field)
        .collect();
    assert_eq!(
        fields,
        vec![NormalizedField::Blocking, NormalizedField::Urgent]
    );
    assert_eq!(classification.normalization_warnings().len(), 2);
}

#[test]
fn a_rule_filled_type_drops_scope_and_steps_that_no_longer_belong() {
    let rules = r#"
        [[rule]]
        name = "Make it a decision"
        type = "decision_required"
    "#;
    let classification = classify_with(
        rules,
        json!({ "scope": "goal", "steps": [{ "text": "Sign in" }] }),
    );
    assert_eq!(classification.scope, None);
    assert!(!classification.keeps_steps);
    let fields: Vec<_> = classification
        .normalizations
        .iter()
        .map(|normalization| normalization.field)
        .collect();
    assert_eq!(fields, vec![NormalizedField::Scope, NormalizedField::Steps]);
}

#[test]
fn scope_survives_on_task_completed() {
    let classification = classify_plain(json!({ "type": "task_completed", "scope": "goal" }));
    assert_eq!(classification.scope, Some(MessageScope::Goal));
}

#[test]
fn declared_wins_field_by_field() {
    let rules = r#"
        [[rule]]
        name = "All"
        type = "task_completed"
        urgent = true
        blocking = "blocking"
    "#;
    let classification = classify_with(
        rules,
        json!({ "type": "decision_required", "urgent": false }),
    );
    assert_eq!(
        classification.message_type.value,
        MessageType::DecisionRequired
    );
    assert_eq!(classification.message_type.source, FieldSource::Declared);
    assert!(!classification.urgent.value);
    assert_eq!(classification.urgent.source, FieldSource::Declared);
    // Only `blocking` was missing: the rule fills it, and it is valid for a decision.
    assert_eq!(classification.blocking.value, Blocking::Blocking);
    assert_eq!(
        classification.blocking.source,
        FieldSource::UserRule("All".to_owned())
    );
}

#[test]
fn a_rule_fills_only_missing_fields_and_later_rules_fill_the_rest() {
    let rules = r#"
        [[rule]]
        name = "First"
        match = { source = "claude-code" }
        type = "decision_required"

        [[rule]]
        name = "Second"
        match = { source = "claude-code" }
        type = "generic"
        urgent = true
        blocking = "deferrable"
    "#;
    let classification = classify_with(rules, json!({}));
    assert_eq!(
        classification.message_type.value,
        MessageType::DecisionRequired
    );
    assert_eq!(
        classification.message_type.source,
        FieldSource::UserRule("First".to_owned())
    );
    assert!(classification.urgent.value);
    assert_eq!(
        classification.urgent.source,
        FieldSource::UserRule("Second".to_owned())
    );
    assert_eq!(classification.blocking.value, Blocking::Deferrable);
    assert_eq!(
        classification.blocking.source,
        FieldSource::UserRule("Second".to_owned())
    );
}

#[test]
fn override_replaces_declared_fields_but_not_an_earlier_rules_value() {
    let rules = r#"
        [[rule]]
        name = "Quiet the docs bot"
        match = { source = "claude-code" }
        type = "generic"
        override = true

        [[rule]]
        name = "Later override"
        match = { source = "claude-code" }
        type = "decision_required"
        override = true
    "#;
    let classification = classify_with(rules, json!({ "type": "decision_required" }));
    assert_eq!(classification.message_type.value, MessageType::Generic);
    assert_eq!(
        classification.message_type.source,
        FieldSource::UserRule("Quiet the docs bot".to_owned())
    );
}

#[test]
fn without_override_a_rule_never_replaces_a_declared_field() {
    let rules = r#"
        [[rule]]
        name = "No override"
        type = "generic"
    "#;
    let classification = classify_with(rules, json!({ "type": "decision_required" }));
    assert_eq!(
        classification.message_type.value,
        MessageType::DecisionRequired
    );
}

#[test]
fn built_in_rules_come_after_user_rules() {
    let rules = r#"
        [[rule]]
        name = "Permission prompts are deferrable here"
        match = { event = "claude-code.permission_request" }
        blocking = "deferrable"
    "#;
    let classification = classify_with(rules, json!({ "event": "claude-code.permission_request" }));
    assert_eq!(classification.blocking.value, Blocking::Deferrable);
    assert!(matches!(
        classification.blocking.source,
        FieldSource::UserRule(_)
    ));
    assert_eq!(
        classification.message_type.value,
        MessageType::DecisionRequired
    );
    assert!(matches!(
        classification.message_type.source,
        FieldSource::BuiltInRule(_)
    ));
}

#[test]
fn defaults_are_generic_false_non_blocking() {
    let classification = classify_plain(json!({}));
    assert_eq!(classification.message_type.value, MessageType::Generic);
    assert!(!classification.urgent.value);
    assert_eq!(classification.blocking.value, Blocking::NonBlocking);
    for source in [
        &classification.message_type.source,
        &classification.urgent.source,
        &classification.blocking.source,
    ] {
        assert_eq!(source, &FieldSource::Default);
    }
}

#[test]
fn steps_make_action_required_the_default_type() {
    let with_steps = classify_plain(json!({ "steps": [{ "text": "Sign in" }] }));
    assert_eq!(with_steps.message_type.value, MessageType::ActionRequired);
    assert_eq!(with_steps.message_type.source, FieldSource::Default);
    assert!(with_steps.keeps_steps);
}

#[test]
fn mute_stores_the_message_as_generic_and_names_the_rule() {
    let rules = r#"
        [[rule]]
        name = "Mute auth success"
        match = { event = "claude-code.notification.auth_success" }
        mute = true
    "#;
    let classification = classify_with(
        rules,
        json!({
            "event": "claude-code.notification.auth_success",
            "type": "decision_required", "urgent": true, "blocking": "blocking"
        }),
    );
    assert_eq!(
        classification.muted_by_rule.as_deref(),
        Some("Mute auth success")
    );
    assert_eq!(classification.message_type.value, MessageType::Generic);
    assert!(!classification.urgent.value);
    assert_eq!(classification.blocking.value, Blocking::NonBlocking);
}

#[test]
fn mute_false_and_a_non_matching_mute_rule_do_nothing() {
    let rules = r#"
        [[rule]]
        name = "Not muted"
        mute = false
        [[rule]]
        name = "Other source"
        match = { source = "codex" }
        mute = true
    "#;
    assert_eq!(classify_with(rules, json!({})).muted_by_rule, None);
}

#[test]
fn match_keys_all_must_hold() {
    let rules = r#"
        [[rule]]
        name = "Work permission prompts"
        match = { source = "claude-code", event = "claude-code.permission_*", cwd = "/Users/me/Work/**", title = "^Allow ", metadata.toolName = "Bash" }
        urgent = true
        override = true
    "#;
    let matching = json!({
        "event": "claude-code.permission_request", "title": "Allow Bash to run tests?",
        "conversation": { "id": "c", "cwd": "/Users/me/Work/api/server" },
        "metadata": { "toolName": "Bash" }
    });
    assert!(classify_with(rules, matching.clone()).urgent.value);
    let table = [
        ("source", json!({ "source": { "agent": "codex" } })),
        ("event", json!({ "event": "claude-code.stop" })),
        (
            "cwd",
            json!({ "conversation": { "id": "c", "cwd": "/Users/me/Play/api" } }),
        ),
        ("title", json!({ "title": "Run something" })),
        ("metadata", json!({ "metadata": { "toolName": "Edit" } })),
    ];
    for (field, change) in table {
        let mut message = matching.clone();
        for (key, value) in change.as_object().unwrap() {
            message[key] = value.clone();
        }
        assert!(
            !classify_with(rules, message).urgent.value,
            "{field} must be able to stop the match"
        );
    }
}

#[test]
fn a_rule_with_no_match_keys_matches_everything() {
    let rules = "[[rule]]\nname = \"All\"\nurgent = true\ntype = \"decision_required\"\n";
    assert!(classify_with(rules, json!({})).urgent.value);
}

#[test]
fn cwd_globs_expand_a_leading_tilde_only_with_a_home_directory() {
    let rules = "[[rule]]\nname = \"Work\"\nmatch = { cwd = \"~/Work/**\" }\nurgent = true\ntype = \"decision_required\"\n";
    let message = input(json!({ "conversation": { "id": "c", "cwd": "/Users/me/Work/api" } }));
    let with_home = RuleSet::parse_with_home(rules, "/Users/me").unwrap();
    assert!(
        classify(&message, &with_home, BuiltInRules::load())
            .urgent
            .value
    );
    let without_home = RuleSet::parse(rules).unwrap();
    assert!(
        !classify(&message, &without_home, BuiltInRules::load())
            .urgent
            .value
    );
}

#[test]
fn glob_star_does_not_cross_directories() {
    let source = "[[rule]]\nname = \"Shallow\"\nmatch = { cwd = \"/work/*\" }\nurgent = true\ntype = \"decision_required\"\n";
    let shallow = input(json!({ "conversation": { "id": "c", "cwd": "/work/api" } }));
    let deep = input(json!({ "conversation": { "id": "c", "cwd": "/work/api/server" } }));
    let rules = RuleSet::parse(source).unwrap();
    assert!(
        classify(&shallow, &rules, BuiltInRules::load())
            .urgent
            .value
    );
    assert!(!classify(&deep, &rules, BuiltInRules::load()).urgent.value);
}

#[test]
fn invalid_toml_is_an_error_value_with_a_line_number() {
    let error = RuleSet::parse("[[rule]]\nname = \"Broken\"\ntype = \n").unwrap_err();
    assert!(matches!(error, RuleParseError::Syntax { .. }));
    assert_eq!(error.line(), Some(3));
}

#[test]
fn invalid_rules_are_error_values_naming_the_rule() {
    let cases = [
        (
            "bad regex",
            "[[rule]]\nname = \"R\"\nmatch = { title = \"(\" }\n",
        ),
        (
            "bad glob",
            "[[rule]]\nname = \"R\"\nmatch = { cwd = \"[\" }\n",
        ),
        (
            "unknown placeholder",
            "[[rule]]\nname = \"R\"\ntopic = { key = \"x:{nope}\" }\n",
        ),
    ];
    for (label, source) in cases {
        let error = RuleSet::parse(source).unwrap_err();
        assert!(
            matches!(&error, RuleParseError::InvalidRule { rule, .. } if rule == "R"),
            "{label}: {error:?}"
        );
        assert_eq!(error.line(), Some(1), "{label}");
    }
}

#[test]
fn unknown_keys_and_values_are_errors() {
    for source in [
        "[[rule]]\nname = \"R\"\ntyp = \"generic\"\n",
        "[[rule]]\nname = \"R\"\ntype = \"loud\"\n",
        "[[rule]]\ntype = \"generic\"\n",
        "[[rule]]\nname = \"R\"\nmatch = { colour = \"red\" }\n",
        "[[other]]\nname = \"R\"\n",
    ] {
        assert!(RuleSet::parse(source).is_err(), "{source}");
    }
}

#[test]
fn an_empty_file_is_an_empty_rule_set() {
    assert!(RuleSet::parse("").unwrap().is_empty());
    assert_eq!(RuleSet::parse("# only a comment\n").unwrap().len(), 0);
}

#[test]
fn the_example_rules_of_the_architecture_parse() {
    let rules = r#"
        [[rule]]
        name = "Quiet the docs bot"
        match = { source = "docs-bot" }
        type = "generic"
        override = true

        [[rule]]
        name = "Permission prompts at work are urgent"
        match = { source = "claude-code", event = "claude-code.permission_request", cwd = "~/Work/**" }
        urgent = true
        override = true

        [[rule]]
        name = "Mute auth success"
        match = { event = "claude-code.notification.auth_success" }
        mute = true

        [[rule]]
        name = "One topic per project"
        match = { source = "claude-code" }
        topic = { key = "project:{project}", title = "{project}", kind = "project" }
    "#;
    assert_eq!(
        RuleSet::parse_with_home(rules, "/Users/me").unwrap().len(),
        4
    );
}

#[test]
fn metadata_matches_numbers_and_booleans_as_text() {
    let rules = "[[rule]]\nname = \"R\"\nmatch = { metadata.exitCode = 1, metadata.isRetry = true }\nurgent = true\ntype = \"decision_required\"\n";
    let message = json!({ "metadata": { "exitCode": 1, "isRetry": true } });
    assert!(classify_with(rules, message).urgent.value);
}

#[test]
fn field_sources_use_stable_labels() {
    for (source, label) in [
        (FieldSource::Declared, "declared"),
        (FieldSource::Default, "default"),
        (FieldSource::Normalized, "normalized"),
        (FieldSource::UserRule("Quiet".to_owned()), "rule:Quiet"),
        (
            FieldSource::BuiltInRule("Claude Code stop".to_owned()),
            "builtin:Claude Code stop",
        ),
    ] {
        assert_eq!(source.label(), label);
        let json = serde_json::to_value(&source).unwrap();
        assert_eq!(json, json!(label));
        assert_eq!(serde_json::from_value::<FieldSource>(json).unwrap(), source);
    }
}

#[test]
fn validation_rejects_duplicate_action_ids_wrong_version_and_bad_rescinds() {
    let duplicate = input(json!({ "actions": [
        { "id": "a", "label": "A", "kind": "approve" },
        { "id": "a", "label": "B", "kind": "deny" }
    ] }));
    assert_eq!(
        validate_message(&duplicate),
        Err(ValidationError::DuplicateActionId("a".to_owned()))
    );

    let mut wrong_version = input(json!({}));
    wrong_version.version = 2;
    assert_eq!(
        validate_message(&wrong_version),
        Err(ValidationError::UnsupportedVersion(2))
    );

    let rescinds = input(json!({ "rescinds": { "reason": "obsolete" } }));
    assert_eq!(
        validate_message(&rescinds),
        Err(ValidationError::RescindsWithoutCorrelationKeys)
    );

    let expiry =
        input(json!({ "createdAt": "2026-10-24T12:00:00Z", "expiresAt": "2026-10-24T11:00:00Z" }));
    assert_eq!(
        validate_message(&expiry),
        Err(ValidationError::ExpiresBeforeCreated)
    );

    let steps = input(json!({ "type": "action_required", "steps": [] }));
    assert!(matches!(
        validate_message(&steps),
        Err(ValidationError::InvalidSteps(_))
    ));
}
