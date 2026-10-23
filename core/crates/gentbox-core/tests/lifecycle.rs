//! Lifecycle transitions, buttons and Defer (architecture §5.2, ADR 0007).
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;

use gentbox_core::actions::{offered_actions, with_hub_actions};
use gentbox_core::lifecycle::{
    ActionError, ActionPress, DEFAULT_SNOOZE_MINUTES, Lifecycle, LifecycleEvent, LifecycleState,
    ResolutionReason, TransitionError, apply_event, default_time_to_live, press_action,
    take_action, transition,
};
use gentbox_core::message::{ActionKind, Blocking, MessageType};
use gentbox_core::policy::{MacSurface, Policy, decide};
use gentbox_core::rescind::RescindReason;
use gentbox_core::response::{ResponseStatus, response_status};
use serde_json::json;
use support::{NOW, awaiting, calm_context, stored};
use time::Duration;

use LifecycleState::{Acknowledged, Answered, Dismissed, Expired, Open, Rescinded};

fn events() -> Vec<LifecycleEvent> {
    vec![
        LifecycleEvent::Acknowledge,
        LifecycleEvent::Snooze {
            until: NOW + Duration::minutes(15),
        },
        LifecycleEvent::SnoozeEnded,
        LifecycleEvent::Defer,
        LifecycleEvent::Answer,
        LifecycleEvent::Dismiss,
        LifecycleEvent::Rescind(RescindReason::Obsolete),
        LifecycleEvent::Expire,
    ]
}

fn name(event: &LifecycleEvent) -> &'static str {
    match event {
        LifecycleEvent::Acknowledge => "acknowledge",
        LifecycleEvent::Snooze { .. } => "snooze",
        LifecycleEvent::SnoozeEnded => "snooze_ended",
        LifecycleEvent::Defer => "defer",
        LifecycleEvent::Answer => "answer",
        LifecycleEvent::Dismiss => "dismiss",
        LifecycleEvent::Rescind(_) => "rescind",
        LifecycleEvent::Expire => "expire",
    }
}

/// Every allowed transition; everything else must be rejected.
fn allowed(from: LifecycleState, event: &LifecycleEvent) -> Option<LifecycleState> {
    match (from, name(event)) {
        (Open, "acknowledge") => Some(Acknowledged),
        (Open | Acknowledged, "snooze") => Some(Acknowledged),
        (Acknowledged, "snooze_ended") => Some(Open),
        (Open, "defer") => Some(Open),
        (Acknowledged, "defer") => Some(Acknowledged),
        (Open | Acknowledged, "answer") => Some(Answered),
        (Open | Acknowledged, "dismiss") => Some(Dismissed),
        (Open | Acknowledged, "rescind") => Some(Rescinded),
        (Open | Acknowledged, "expire") => Some(Expired),
        _ => None,
    }
}

#[test]
fn every_allowed_transition_is_accepted_and_every_other_rejected() {
    let mut accepted = 0;
    for from in LifecycleState::ALL {
        for event in events() {
            let result = transition(from, &event);
            match allowed(from, &event) {
                Some(to) => {
                    assert_eq!(result, Ok(to), "{from:?} --{}-->", name(&event));
                    accepted += 1;
                }
                None => assert_eq!(
                    result,
                    Err(TransitionError::NotAllowed {
                        from,
                        event: name(&event)
                    }),
                    "{from:?} --{}-->",
                    name(&event)
                ),
            }
        }
    }
    assert_eq!(accepted, 14);
}

#[test]
fn a_snooze_that_ends_returns_an_acknowledged_message_to_open() {
    let lifecycle = Lifecycle::born(MessageType::DecisionRequired);
    let snoozed = apply_event(
        &lifecycle,
        &LifecycleEvent::Snooze {
            until: NOW + Duration::minutes(15),
        },
        NOW,
    )
    .unwrap();
    assert_eq!(snoozed.state, Acknowledged);
    assert_eq!(snoozed.snoozed_until, Some(NOW + Duration::minutes(15)));
    let ended = apply_event(&snoozed, &LifecycleEvent::SnoozeEnded, NOW).unwrap();
    assert_eq!(ended.state, Open);
    assert_eq!(ended.snoozed_until, None);
}

#[test]
fn closing_records_the_resolution_and_clears_the_snooze() {
    let snoozed = Lifecycle {
        state: Acknowledged,
        snoozed_until: Some(NOW + Duration::minutes(5)),
        deferred_at: None,
        resolution: None,
        answer: None,
    };
    for (event, resolution) in [
        (LifecycleEvent::Answer, ResolutionReason::Answered),
        (LifecycleEvent::Dismiss, ResolutionReason::Dismissed),
        (LifecycleEvent::Expire, ResolutionReason::Expired),
        (
            LifecycleEvent::Rescind(RescindReason::Superseded),
            ResolutionReason::Rescinded(RescindReason::Superseded),
        ),
    ] {
        let closed = apply_event(&snoozed, &event, NOW).unwrap();
        assert_eq!(closed.resolution, Some(resolution));
        assert_eq!(closed.snoozed_until, None);
        assert!(closed.state.is_closed());
    }
}

#[test]
fn generic_messages_are_born_acknowledged_and_the_rest_open() {
    for message_type in MessageType::ALL {
        let expected = if message_type == MessageType::Generic {
            Acknowledged
        } else {
            Open
        };
        assert_eq!(Lifecycle::born(message_type).state, expected);
    }
}

#[test]
fn open_and_snooze_never_close_a_message_or_reach_the_sender() {
    let message = awaiting(MessageType::DecisionRequired, false, Blocking::Blocking);
    let opened = take_action(&message, ActionKind::Open, NOW, None).unwrap();
    assert_eq!(opened.sender_status, None);
    assert_eq!(opened.message.lifecycle.state, Acknowledged);
    assert!(opened.message.is_live());

    let snoozed = take_action(&message, ActionKind::Snooze, NOW, Some(30)).unwrap();
    assert_eq!(snoozed.sender_status, None);
    assert_eq!(snoozed.message.lifecycle.state, Acknowledged);
    assert_eq!(
        snoozed.message.lifecycle.snoozed_until,
        Some(NOW + Duration::minutes(30))
    );

    let default_snooze = take_action(&message, ActionKind::Snooze, NOW, None).unwrap();
    assert_eq!(
        default_snooze.message.lifecycle.snoozed_until,
        Some(NOW + Duration::minutes(i64::from(DEFAULT_SNOOZE_MINUTES)))
    );
    assert!(!ActionKind::Open.reaches_sender());
    assert!(!ActionKind::Snooze.reaches_sender());
}

#[test]
fn opening_an_already_acknowledged_message_changes_nothing() {
    let message = awaiting(MessageType::DecisionRequired, false, Blocking::Blocking);
    let once = take_action(&message, ActionKind::Open, NOW, None)
        .unwrap()
        .message;
    let twice = take_action(&once, ActionKind::Open, NOW, None).unwrap();
    assert_eq!(twice.message, once);
}

#[test]
fn answers_close_the_message_and_reach_the_sender() {
    for kind in [
        ActionKind::Approve,
        ActionKind::Deny,
        ActionKind::Choice,
        ActionKind::Reply,
    ] {
        let message = awaiting(MessageType::DecisionRequired, false, Blocking::Blocking);
        let outcome = take_action(&message, kind, NOW, None).unwrap();
        assert_eq!(outcome.message.lifecycle.state, Answered, "{kind:?}");
        assert_eq!(
            outcome.message.lifecycle.resolution,
            Some(ResolutionReason::Answered)
        );
        assert_eq!(outcome.sender_status, Some(ResponseStatus::Answered));
        assert!(kind.is_answer() && kind.reaches_sender());
    }
}

#[test]
fn the_first_answer_wins_and_a_late_one_is_refused() {
    let message = awaiting(MessageType::DecisionRequired, false, Blocking::Blocking);
    let answered = take_action(&message, ActionKind::Approve, NOW, None)
        .unwrap()
        .message;
    for kind in [
        ActionKind::Deny,
        ActionKind::Approve,
        ActionKind::Open,
        ActionKind::Snooze,
        ActionKind::Defer,
    ] {
        assert_eq!(
            take_action(&answered, kind, NOW, None),
            Err(ActionError::Closed {
                state: Answered,
                resolution: Some(ResolutionReason::Answered)
            }),
            "{kind:?}"
        );
    }
}

#[test]
fn a_rescinded_message_rejects_later_answers_and_rescinding_never_answers() {
    let message = awaiting(MessageType::DecisionRequired, false, Blocking::Blocking);
    let rescinded = apply_event(
        &message.lifecycle,
        &LifecycleEvent::Rescind(RescindReason::ResolvedElsewhere),
        NOW,
    )
    .unwrap();
    let mut closed = message;
    closed.lifecycle = rescinded;
    assert_eq!(closed.lifecycle.state, Rescinded);
    assert_ne!(
        response_status(closed.lifecycle.state, false),
        ResponseStatus::Answered
    );
    assert_eq!(
        take_action(&closed, ActionKind::Approve, NOW, None),
        Err(ActionError::Closed {
            state: Rescinded,
            resolution: Some(ResolutionReason::Rescinded(
                RescindReason::ResolvedElsewhere
            )),
        })
    );
}

#[test]
fn defer_is_only_allowed_on_deferrable_messages() {
    for (blocking, allowed) in [
        (Blocking::Deferrable, true),
        (Blocking::Blocking, false),
        (Blocking::NonBlocking, false),
    ] {
        let message = awaiting(MessageType::DecisionRequired, false, blocking);
        let result = take_action(&message, ActionKind::Defer, NOW, None);
        assert_eq!(result.is_ok(), allowed, "{blocking:?}");
        if !allowed {
            assert_eq!(result, Err(ActionError::NotDeferrable));
        }
    }
}

#[test]
fn defer_turns_blocking_into_non_blocking_and_the_policy_follows() {
    let message = awaiting(MessageType::DecisionRequired, false, Blocking::Deferrable);
    let policy = Policy::default();
    let before = decide(&message, &calm_context(), &policy);
    assert_eq!(before.mac, MacSurface::PersistentPopup);
    assert!(before.offer_defer);
    assert!(before.reping.is_some());

    let outcome = take_action(&message, ActionKind::Defer, NOW, None).unwrap();
    assert_eq!(outcome.sender_status, Some(ResponseStatus::Deferred));
    assert_eq!(outcome.message.blocking, Blocking::NonBlocking);
    assert_eq!(outcome.message.lifecycle.deferred_at, Some(NOW));
    assert!(
        outcome.message.lifecycle.state.is_live(),
        "the message stays open"
    );

    // "After Defer: as non_blocking": the popup closes and re-pings stop.
    let after = decide(&outcome.message, &calm_context(), &policy);
    assert_eq!(after.mac, MacSurface::Banner);
    assert_eq!(after.reping, None);
    assert!(!after.offer_defer);
}

#[test]
fn the_waiter_hears_deferred_once_then_pending_and_a_real_answer_still_closes() {
    let message = awaiting(MessageType::DecisionRequired, false, Blocking::Deferrable);
    let deferred = take_action(&message, ActionKind::Defer, NOW, None)
        .unwrap()
        .message;
    // First poll after the Defer: `deferred`; the hub then marks it reported.
    assert_eq!(
        response_status(deferred.lifecycle.state, true),
        ResponseStatus::Deferred
    );
    assert_eq!(
        response_status(deferred.lifecycle.state, false),
        ResponseStatus::Pending
    );
    // A later real answer still closes the message.
    let answered = take_action(&deferred, ActionKind::Approve, NOW, None).unwrap();
    assert_eq!(answered.message.lifecycle.state, Answered);
    assert_eq!(answered.sender_status, Some(ResponseStatus::Answered));
    assert_eq!(response_status(Answered, false), ResponseStatus::Answered);
}

#[test]
fn defer_works_on_an_acknowledged_deferrable_message() {
    let message = awaiting(MessageType::DecisionRequired, false, Blocking::Deferrable);
    let acknowledged = take_action(&message, ActionKind::Open, NOW, None)
        .unwrap()
        .message;
    let deferred = take_action(&acknowledged, ActionKind::Defer, NOW, None).unwrap();
    assert_eq!(deferred.message.lifecycle.state, Acknowledged);
    assert_eq!(deferred.message.blocking, Blocking::NonBlocking);
}

#[test]
fn lifecycle_maps_to_the_long_poll_status_table() {
    let rows = [
        (Open, false, ResponseStatus::Pending),
        (Acknowledged, false, ResponseStatus::Pending),
        (Open, true, ResponseStatus::Deferred),
        (Acknowledged, true, ResponseStatus::Deferred),
        (Answered, false, ResponseStatus::Answered),
        (Dismissed, false, ResponseStatus::Dismissed),
        (Rescinded, false, ResponseStatus::Rescinded),
        (Expired, false, ResponseStatus::Expired),
        // A closed message never reports a stale deferral.
        (Answered, true, ResponseStatus::Answered),
        (Rescinded, true, ResponseStatus::Rescinded),
    ];
    for (state, deferral_unreported, expected) in rows {
        assert_eq!(
            response_status(state, deferral_unreported),
            expected,
            "{state:?} {deferral_unreported}"
        );
    }
}

#[test]
fn default_time_to_live_follows_the_table() {
    let rows = [
        (
            MessageType::DecisionRequired,
            Blocking::Blocking,
            Duration::hours(24),
        ),
        (
            MessageType::DecisionRequired,
            Blocking::Deferrable,
            Duration::hours(24),
        ),
        (
            MessageType::ActionRequired,
            Blocking::Blocking,
            Duration::hours(24),
        ),
        (
            MessageType::DecisionRequired,
            Blocking::NonBlocking,
            Duration::days(7),
        ),
        (
            MessageType::ActionRequired,
            Blocking::NonBlocking,
            Duration::days(7),
        ),
        (
            MessageType::TaskCompleted,
            Blocking::NonBlocking,
            Duration::days(7),
        ),
        (
            MessageType::Generic,
            Blocking::NonBlocking,
            Duration::hours(24),
        ),
    ];
    for (message_type, blocking, expected) in rows {
        assert_eq!(default_time_to_live(message_type, blocking), expected);
    }
    let message = stored(
        MessageType::TaskCompleted,
        false,
        Blocking::NonBlocking,
        json!({}),
    );
    assert_eq!(message.expires_at, message.received_at + Duration::days(7));
    let explicit = stored(
        MessageType::TaskCompleted,
        false,
        Blocking::NonBlocking,
        json!({ "expiresAt": "2026-10-24T18:00:00Z" }),
    );
    assert_eq!(
        explicit.expires_at,
        time::macros::datetime!(2026-10-24 18:00:00 UTC)
    );
}

#[test]
fn the_hub_adds_done_and_defer_buttons() {
    let action_required = stored(
        MessageType::ActionRequired,
        false,
        Blocking::Deferrable,
        json!({}),
    );
    let kinds: Vec<_> = action_required
        .actions
        .iter()
        .map(|action| action.kind)
        .collect();
    assert_eq!(kinds, vec![ActionKind::Done, ActionKind::Defer]);

    let decision = stored(
        MessageType::DecisionRequired,
        false,
        Blocking::Blocking,
        json!({}),
    );
    assert!(decision.actions.is_empty());

    // Declared ones are kept and not duplicated.
    let declared = with_hub_actions(
        serde_json::from_value(json!([{ "id": "done", "label": "Finished", "kind": "done" }]))
            .unwrap(),
        MessageType::ActionRequired,
        Blocking::Blocking,
    );
    assert_eq!(declared.len(), 1);
    assert_eq!(declared[0].label, "Finished");

    // A taken id gets a numbered suffix.
    let clash = with_hub_actions(
        serde_json::from_value(json!([{ "id": "done", "label": "Open", "kind": "open" }])).unwrap(),
        MessageType::ActionRequired,
        Blocking::Blocking,
    );
    let ids: Vec<_> = clash.iter().map(|action| action.id.as_str()).collect();
    assert_eq!(ids, vec!["done", "done-2"]);
}

#[test]
fn answer_buttons_need_a_live_waiter_and_defer_only_while_deferrable() {
    let message = stored(
        MessageType::DecisionRequired,
        false,
        Blocking::Deferrable,
        json!({ "awaitsResponse": true, "actions": [
            { "id": "allow", "label": "Allow", "kind": "approve" },
            { "id": "open", "label": "Open session", "kind": "open" }
        ] }),
    );
    let kinds = |waiter| -> Vec<ActionKind> {
        offered_actions(&message, waiter)
            .iter()
            .map(|action| action.kind)
            .collect()
    };
    assert_eq!(
        kinds(true),
        vec![ActionKind::Approve, ActionKind::Open, ActionKind::Defer]
    );
    assert_eq!(kinds(false), vec![ActionKind::Open]);

    let deferred = take_action(&message, ActionKind::Defer, NOW, None)
        .unwrap()
        .message;
    let after: Vec<_> = offered_actions(&deferred, true)
        .iter()
        .map(|action| action.kind)
        .collect();
    assert_eq!(
        after,
        vec![ActionKind::Approve, ActionKind::Open],
        "Defer is gone once deferred"
    );

    let closed = take_action(&message, ActionKind::Approve, NOW, None)
        .unwrap()
        .message;
    assert!(offered_actions(&closed, true).is_empty());
}

#[test]
fn an_answer_the_message_does_not_offer_is_refused() {
    // Approve on a completion, Done on a decision, a reply on a message
    // without a reply button: none closes the message.
    let cases = [
        (
            stored(
                MessageType::TaskCompleted,
                false,
                Blocking::NonBlocking,
                json!({}),
            ),
            ActionKind::Approve,
        ),
        (
            stored(
                MessageType::Generic,
                false,
                Blocking::NonBlocking,
                json!({}),
            ),
            ActionKind::Deny,
        ),
        (
            awaiting(MessageType::DecisionRequired, false, Blocking::Blocking),
            ActionKind::Done,
        ),
        (
            stored(
                MessageType::DecisionRequired,
                false,
                Blocking::Blocking,
                json!({ "awaitsResponse": true, "actions": [{ "id": "allow", "label": "Allow", "kind": "approve" }] }),
            ),
            ActionKind::Reply,
        ),
    ];
    for (message, kind) in cases {
        assert_eq!(
            take_action(&message, kind, NOW, None),
            Err(ActionError::UnknownAction),
            "{kind:?} on {:?}",
            message.message_type
        );
    }
}

#[test]
fn a_defer_press_must_name_the_defer_button() {
    let message = awaiting(MessageType::DecisionRequired, false, Blocking::Deferrable);
    let mut press = ActionPress::of_kind(ActionKind::Defer);
    press.action_id = Some("allow".to_owned());
    assert_eq!(
        press_action(&message, &press, NOW),
        Err(ActionError::UnknownAction)
    );
    press.action_id = Some(gentbox_core::actions::DEFER_ACTION_ID.to_owned());
    assert!(press_action(&message, &press, NOW).is_ok());
}
