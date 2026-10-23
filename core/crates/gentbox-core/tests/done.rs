//! Done on `action_required` (ADR 0011): always offered, closes locally, and is
//! recorded so a later poll still hears it, with no second way to finish it.
#![allow(clippy::unwrap_used, clippy::expect_used)]
// The text scan at the end reads the docs and the crate source.
#![allow(clippy::disallowed_methods)]
mod support;

use std::fs;
use std::path::{Path, PathBuf};

use gentbox_core::actions::offered_actions;
use gentbox_core::classify::{BuiltInRules, RuleSet};
use gentbox_core::ids::{MessageId, ThreadId, TopicId};
use gentbox_core::lifecycle::{
    ActionError, ActionPress, LifecycleState, ResolutionReason, dismiss, press_action, take_action,
};
use gentbox_core::message::{ActionKind, Blocking, Message, MessageType};
use gentbox_core::prepare::{IngestSettings, prepare_message};
use gentbox_core::response::{ActionResponse, AnsweredVia, ResponseStatus};
use serde_json::json;
use support::{NOW, awaiting, stored, valid_combinations};
use time::Duration;

/// A message as the hub would store it, from a Claude Code event.
fn from_event(event: &str) -> Message {
    prepare_message(
        support::input(json!({ "event": event })),
        &RuleSet::empty(),
        BuiltInRules::load(),
        IngestSettings::default(),
    )
    .unwrap()
    .into_message(
        MessageId::new("01JBM3Z8Q7Y4K2N6P0R5T9VWXA"),
        NOW,
        TopicId::new("T"),
        ThreadId::new("H"),
    )
}

fn offers_done(message: &Message, has_live_waiter: bool) -> bool {
    offered_actions(message, has_live_waiter)
        .iter()
        .any(|action| action.kind == ActionKind::Done)
}

#[test]
fn every_action_required_combination_offers_done_with_and_without_a_waiter() {
    let mut checked = 0;
    for (message_type, urgent, blocking) in valid_combinations() {
        if message_type != MessageType::ActionRequired {
            continue;
        }
        for message in [
            awaiting(message_type, urgent, blocking),
            stored(message_type, urgent, blocking, json!({})),
        ] {
            for has_live_waiter in [false, true] {
                assert!(
                    offers_done(&message, has_live_waiter),
                    "urgent={urgent} {blocking:?} waiter={has_live_waiter}"
                );
                checked += 1;
            }
        }
    }
    assert_eq!(checked, 6 * 2 * 2);
}

#[test]
fn the_claude_code_action_rows_offer_done_without_any_waiter() {
    for event in [
        "claude-code.stop_failure",
        "claude-code.notification.quota_auto_resume_stale",
        "claude-code.notification.elicitation_url_dialog",
    ] {
        let message = from_event(event);
        assert_eq!(message.message_type, MessageType::ActionRequired, "{event}");
        assert_eq!(message.blocking, Blocking::Blocking, "{event}");
        assert!(
            !message.awaits_response,
            "{event}: the hook never waits here"
        );
        assert!(offers_done(&message, false), "{event}");
        let kinds: Vec<_> = offered_actions(&message, false)
            .iter()
            .map(|a| a.kind)
            .collect();
        assert_eq!(
            kinds,
            vec![ActionKind::Done],
            "{event}: only Done, nothing else pretends"
        );
    }
}

#[test]
fn done_is_not_offered_without_a_waiter_on_other_types() {
    for message_type in [MessageType::DecisionRequired, MessageType::TaskCompleted] {
        let message = stored(
            message_type,
            false,
            Blocking::NonBlocking,
            json!({ "actions": [{ "id": "done", "label": "Done", "kind": "done" }] }),
        );
        assert!(!offers_done(&message, false), "{message_type:?}");
        assert!(
            offers_done(&message, true),
            "{message_type:?} with a waiter"
        );
    }
}

#[test]
fn done_without_a_waiter_closes_the_item_locally_and_is_recorded() {
    let message = from_event("claude-code.stop_failure");
    let outcome = take_action(&message, ActionKind::Done, NOW, None).unwrap();
    assert_eq!(outcome.message.lifecycle.state, LifecycleState::Answered);
    assert_eq!(
        outcome.message.lifecycle.resolution,
        Some(ResolutionReason::Answered)
    );
    let recorded = outcome.message.lifecycle.answer.clone().unwrap();
    assert_eq!(recorded.action_id, "done");
    assert_eq!(recorded.action_kind, ActionKind::Done);
    assert_eq!(recorded.answered_at, NOW);
    // With a live waiter the same outcome is delivered at once.
    assert_eq!(outcome.sender_status, Some(ResponseStatus::Answered));
}

#[test]
fn a_later_poll_after_a_waiterless_done_returns_done() {
    let message = from_event("claude-code.notification.quota_auto_resume_stale");
    let mut press = ActionPress::of_kind(ActionKind::Done);
    press.via = Some(AnsweredVia::Mac);
    let later = NOW + Duration::minutes(40);
    let done = press_action(&message, &press, later).unwrap().message;

    // The agent polls only now: it finds the recorded answer.
    let response = ActionResponse::for_message(&done, false);
    assert_eq!(response.status, ResponseStatus::Answered);
    assert_eq!(response.action_kind, Some(ActionKind::Done));
    assert_eq!(response.action_id.as_deref(), Some("done"));
    assert_eq!(response.answered_via, Some(AnsweredVia::Mac));
    assert_eq!(response.answered_at, Some(later));
    assert_eq!(response.message_id, done.id);

    // Polling again says the same; and the wire shape is the schema's.
    assert_eq!(ActionResponse::for_message(&done, false), response);
    let json = serde_json::to_value(&response).unwrap();
    assert_eq!(json["status"], "answered");
    assert_eq!(json["actionKind"], "done");
    assert!(serde_json::from_value::<ActionResponse>(json).is_ok());
}

#[test]
fn a_poll_before_done_is_pending_and_other_states_carry_no_answer() {
    let message = from_event("claude-code.stop_failure");
    let pending = ActionResponse::for_message(&message, false);
    assert_eq!(pending.status, ResponseStatus::Pending);
    assert_eq!((pending.action_id, pending.action_kind), (None, None));

    let rescinded = gentbox_core::lifecycle::apply_event(
        &message.lifecycle,
        &gentbox_core::lifecycle::LifecycleEvent::Rescind(
            gentbox_core::rescind::RescindReason::Obsolete,
        ),
        NOW,
    )
    .unwrap();
    let mut withdrawn = message.clone();
    withdrawn.lifecycle = rescinded;
    let response = ActionResponse::for_message(&withdrawn, false);
    assert_eq!(response.status, ResponseStatus::Rescinded);
    assert_eq!(
        response.rescind_reason,
        Some(gentbox_core::rescind::RescindReason::Obsolete)
    );
    assert_eq!(response.action_kind, None);
}

#[test]
fn answers_record_their_value_and_id() {
    let message = stored(
        MessageType::DecisionRequired,
        false,
        Blocking::Blocking,
        json!({ "awaitsResponse": true, "actions": [
            { "id": "keep", "label": "Keep Relay", "kind": "choice", "value": "Relay" },
            { "id": "other", "label": "Other…", "kind": "reply" }
        ] }),
    );
    let mut press = ActionPress::of_kind(ActionKind::Reply);
    press.action_id = Some("other".to_owned());
    press.value = Some("PushChannel".to_owned());
    press.via = Some(AnsweredVia::Phone);
    let answered = press_action(&message, &press, NOW).unwrap().message;
    let response = ActionResponse::for_message(&answered, false);
    assert_eq!(response.action_id.as_deref(), Some("other"));
    assert_eq!(response.action_kind, Some(ActionKind::Reply));
    assert_eq!(response.value.as_deref(), Some("PushChannel"));
    assert_eq!(response.answered_via, Some(AnsweredVia::Phone));
}

#[test]
fn a_pressed_action_id_must_exist_with_that_kind() {
    let message = awaiting(MessageType::DecisionRequired, false, Blocking::Blocking);
    let mut press = ActionPress::of_kind(ActionKind::Approve);
    press.action_id = Some("nope".to_owned());
    assert_eq!(
        press_action(&message, &press, NOW),
        Err(ActionError::UnknownAction)
    );

    let with_buttons = stored(
        MessageType::DecisionRequired,
        false,
        Blocking::Blocking,
        json!({ "awaitsResponse": true, "actions": [{ "id": "allow", "label": "Allow", "kind": "approve" }] }),
    );
    let mut wrong_kind = ActionPress::of_kind(ActionKind::Deny);
    wrong_kind.action_id = Some("allow".to_owned());
    assert_eq!(
        press_action(&with_buttons, &wrong_kind, NOW),
        Err(ActionError::UnknownAction)
    );
    let mut right = ActionPress::of_kind(ActionKind::Approve);
    right.action_id = Some("allow".to_owned());
    assert!(press_action(&with_buttons, &right, NOW).is_ok());
}

#[test]
fn a_late_second_done_is_refused_like_any_late_answer() {
    let message = from_event("claude-code.stop_failure");
    let done = take_action(&message, ActionKind::Done, NOW, None)
        .unwrap()
        .message;
    assert!(matches!(
        take_action(&done, ActionKind::Done, NOW, None),
        Err(ActionError::Closed {
            state: LifecycleState::Answered,
            ..
        })
    ));
    assert!(offered_actions(&done, true).is_empty());
}

#[test]
fn an_action_required_message_is_never_cleared_only_closed_with_done() {
    let action = from_event("claude-code.stop_failure");
    assert_eq!(dismiss(&action, NOW), Err(ActionError::UseDoneInstead));
    for (message_type, blocking) in [
        (MessageType::Generic, Blocking::NonBlocking),
        (MessageType::TaskCompleted, Blocking::NonBlocking),
        (MessageType::DecisionRequired, Blocking::Blocking),
    ] {
        let message = stored(message_type, false, blocking, json!({}));
        let cleared = dismiss(&message, NOW).unwrap();
        assert_eq!(
            cleared.lifecycle.state,
            LifecycleState::Dismissed,
            "{message_type:?}"
        );
    }
}

// --- no second way to finish an item: the retired button name is gone ---

fn files_under(directory: &Path, extensions: &[&str], found: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files_under(&path, extensions, found);
        } else if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extensions.contains(&extension))
        {
            found.push(path);
        }
    }
}

#[test]
fn the_retired_clear_button_name_is_gone_from_the_crate_and_the_docs() {
    let phrases = [
        ["mark", "done"].join(" "),
        ["mark", "done"].join("_"),
        ["mark", "done"].join("-"),
    ];
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    files_under(&manifest.join("src"), &["rs", "toml"], &mut files);
    files_under(&manifest.join("tests"), &["rs"], &mut files);
    files_under(&manifest.join("../../../docs"), &["md", "json"], &mut files);
    assert!(files.len() > 40, "found {} files", files.len());
    let mut hits = Vec::new();
    for file in files {
        let text = fs::read_to_string(&file).unwrap().to_lowercase();
        for phrase in &phrases {
            if text.contains(phrase.as_str()) {
                hits.push(format!("{}: `{phrase}`", file.display()));
            }
        }
    }
    assert!(hits.is_empty(), "{}", hits.join("\n"));
}
