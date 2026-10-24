//! Coalescing, dedupe, supersede and the implicit rescinds (architecture §7).
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;

use gentbox_core::coalesce::{
    CoalesceInput, CoalesceOutcome, DuplicateKind, IDLE_CORRELATION_KEY, coalesce,
    progress_rescissions,
};
use gentbox_core::ids::{ConversationKey, MessageId};
use gentbox_core::item::OpenItem;
use gentbox_core::lifecycle::LifecycleState;
use gentbox_core::message::{Blocking, Message, MessageType};
use gentbox_core::rescind::{ChannelEffect, RescindReason, RescindRequest};
use serde_json::json;
use support::{NOW, item_of, stored_at};
use time::Duration;

use Blocking::{Blocking as Blocks, Deferrable, NonBlocking};
use MessageType::{ActionRequired, DecisionRequired, Generic, TaskCompleted};

fn run(incoming: &Message, items: &[OpenItem]) -> CoalesceOutcome {
    coalesce(&CoalesceInput {
        incoming,
        rescinds: None,
        idempotent_match: None,
        open_items: items,
        will_have_live_waiter: incoming.awaits_response,
        has_phone_channel: false,
    })
}

fn message(
    id: &str,
    message_type: MessageType,
    blocking: Blocking,
    extra: serde_json::Value,
) -> Message {
    stored_at(id, NOW, message_type, false, blocking, extra)
}

fn rescinded_ids(outcome: &CoalesceOutcome) -> Vec<(String, RescindReason)> {
    let rescissions = match outcome {
        CoalesceOutcome::New { rescissions } | CoalesceOutcome::Superseded { rescissions, .. } => {
            rescissions
        }
        _ => return Vec::new(),
    };
    rescissions
        .iter()
        .map(|rescission| (rescission.message_id.to_string(), rescission.reason))
        .collect()
}

#[test]
fn an_idempotency_key_match_returns_the_stored_message_and_changes_nothing() {
    let incoming = message("NEW", DecisionRequired, Blocks, json!({}));
    let existing = MessageId::new("STORED");
    let outcome = coalesce(&CoalesceInput {
        incoming: &incoming,
        rescinds: None,
        idempotent_match: Some(&existing),
        open_items: &[],
        will_have_live_waiter: false,
        has_phone_channel: false,
    });
    assert_eq!(
        outcome,
        CoalesceOutcome::Duplicate {
            of: existing,
            kind: DuplicateKind::Idempotent,
            rescissions: vec![]
        }
    );
}

#[test]
fn a_message_without_a_matching_key_is_new() {
    let incoming = message(
        "NEW",
        DecisionRequired,
        Blocks,
        json!({ "correlationKey": "a" }),
    );
    let other = message(
        "OLD",
        DecisionRequired,
        Blocks,
        json!({ "correlationKey": "b", "awaitsResponse": true }),
    );
    assert_eq!(
        run(&incoming, &[item_of(&other, false)]),
        CoalesceOutcome::New {
            rescissions: vec![]
        }
    );
}

#[test]
fn the_same_correlation_key_supersedes_in_place() {
    let old = message(
        "OLD",
        DecisionRequired,
        Blocks,
        json!({ "correlationKey": "ask", "title": "Which name?" }),
    );
    let incoming = stored_at(
        "NEW",
        NOW + Duration::minutes(2),
        DecisionRequired,
        false,
        Blocks,
        json!({ "correlationKey": "ask", "title": "Which name should it have?" }),
    );
    match run(&incoming, &[item_of(&old, false)]) {
        CoalesceOutcome::Superseded {
            replaces,
            pings,
            rescissions,
        } => {
            assert_eq!(replaces.as_str(), "OLD");
            assert!(!pings, "no re-ping unless louder");
            assert_eq!(rescissions.len(), 1);
            assert_eq!(rescissions[0].reason, RescindReason::Superseded);
            assert!(
                rescissions[0]
                    .effects
                    .contains(&ChannelEffect::FoldIntoSuccessor(MessageId::new("NEW")))
            );
        }
        other => panic!("expected a supersede, got {other:?}"),
    }
}

#[test]
fn a_louder_superseding_message_pings() {
    let table = [
        (
            "urgent over non-urgent",
            (DecisionRequired, false, Deferrable),
            (DecisionRequired, true, Deferrable),
            true,
        ),
        (
            "higher blocking level",
            (DecisionRequired, false, NonBlocking),
            (DecisionRequired, false, Blocks),
            true,
        ),
        (
            "deferrable to blocking",
            (DecisionRequired, false, Deferrable),
            (DecisionRequired, false, Blocks),
            true,
        ),
        (
            "same level",
            (DecisionRequired, false, Blocks),
            (DecisionRequired, false, Blocks),
            false,
        ),
        (
            "quieter blocking",
            (DecisionRequired, false, Blocks),
            (DecisionRequired, false, Deferrable),
            false,
        ),
        (
            "urgent to non-urgent",
            (DecisionRequired, true, Blocks),
            (DecisionRequired, false, Blocks),
            false,
        ),
        (
            "both urgent",
            (DecisionRequired, true, Blocks),
            (DecisionRequired, true, Blocks),
            false,
        ),
    ];
    for (
        label,
        (old_type, old_urgent, old_blocking),
        (new_type, new_urgent, new_blocking),
        expected,
    ) in table
    {
        let old = stored_at(
            "OLD",
            NOW,
            old_type,
            old_urgent,
            old_blocking,
            json!({ "correlationKey": "k", "title": "Old" }),
        );
        let incoming = stored_at(
            "NEW",
            NOW + Duration::minutes(1),
            new_type,
            new_urgent,
            new_blocking,
            json!({ "correlationKey": "k", "title": "New" }),
        );
        match run(&incoming, &[item_of(&old, false)]) {
            CoalesceOutcome::Superseded { pings, .. } => assert_eq!(pings, expected, "{label}"),
            other => panic!("{label}: expected a supersede, got {other:?}"),
        }
    }
}

#[test]
fn a_message_without_a_live_waiter_folds_into_one_with_a_live_waiter_by_key() {
    let old = message(
        "OLD",
        DecisionRequired,
        Blocks,
        json!({ "correlationKey": "permission:main:abc", "awaitsResponse": true }),
    );
    let incoming = message(
        "NEW",
        DecisionRequired,
        Blocks,
        json!({ "correlationKey": "permission:main:abc" }),
    );
    assert_eq!(
        run(&incoming, &[item_of(&old, true)]),
        CoalesceOutcome::Folded {
            into: MessageId::new("OLD"),
            rescissions: vec![]
        }
    );
}

#[test]
fn a_message_that_will_wait_supersedes_even_a_live_waiter() {
    let old = message(
        "OLD",
        DecisionRequired,
        Blocks,
        json!({ "correlationKey": "k", "awaitsResponse": true, "title": "Old" }),
    );
    let incoming = stored_at(
        "NEW",
        NOW + Duration::minutes(1),
        DecisionRequired,
        false,
        Blocks,
        json!({ "correlationKey": "k", "awaitsResponse": true, "title": "New" }),
    );
    let outcome = run(&incoming, &[item_of(&old, true)]);
    let CoalesceOutcome::Superseded { rescissions, .. } = outcome else {
        panic!("expected a supersede")
    };
    assert!(
        rescissions[0]
            .effects
            .contains(&ChannelEffect::WakeWaiter(RescindReason::Superseded))
    );
}

#[test]
fn a_blocking_message_folds_into_a_blocking_item_with_a_live_waiter_without_a_key() {
    let waiting = message(
        "OLD",
        DecisionRequired,
        Blocks,
        json!({ "correlationKey": "permission:main:abc", "awaitsResponse": true }),
    );
    for (blocking, folds) in [(Blocks, true), (Deferrable, true), (NonBlocking, false)] {
        let incoming = message("NEW", DecisionRequired, blocking, json!({}));
        let outcome = run(&incoming, &[item_of(&waiting, true)]);
        assert_eq!(
            matches!(outcome, CoalesceOutcome::Folded { .. }),
            folds,
            "{blocking:?}"
        );
    }
    // Blocking-into-blocking also folds across types (a notification into a permission prompt).
    let incoming = message("NEW", ActionRequired, Blocks, json!({}));
    assert!(matches!(
        run(&incoming, &[item_of(&waiting, true)]),
        CoalesceOutcome::Folded { .. }
    ));
    // Without a live waiter nothing folds.
    let incoming = message("NEW", DecisionRequired, Blocks, json!({}));
    assert!(matches!(
        run(&incoming, &[item_of(&waiting, false)]),
        CoalesceOutcome::New { .. }
    ));
}

#[test]
fn folding_never_applies_to_a_message_that_will_wait() {
    let waiting = message(
        "OLD",
        DecisionRequired,
        Blocks,
        json!({ "correlationKey": "permission:main:abc", "awaitsResponse": true }),
    );
    let incoming = message(
        "NEW",
        DecisionRequired,
        Blocks,
        json!({ "correlationKey": "permission:main:def", "awaitsResponse": true }),
    );
    assert!(matches!(
        run(&incoming, &[item_of(&waiting, true)]),
        CoalesceOutcome::New { .. }
    ));
}

#[test]
fn parallel_permission_keys_coexist() {
    let first = message(
        "A",
        DecisionRequired,
        Blocks,
        json!({ "correlationKey": "permission:main:3f9a0c2d71be", "awaitsResponse": true }),
    );
    let second = message(
        "B",
        DecisionRequired,
        Blocks,
        json!({ "correlationKey": "permission:agent-7:3f9a0c2d71be", "awaitsResponse": true }),
    );
    let third = message(
        "C",
        DecisionRequired,
        Blocks,
        json!({ "correlationKey": "permission:main:8c2e41f0a9d3", "awaitsResponse": true }),
    );
    let items = [item_of(&first, true), item_of(&second, true)];
    assert!(matches!(run(&third, &items), CoalesceOutcome::New { .. }));
    assert!(matches!(
        run(&second, &items[..1]),
        CoalesceOutcome::New { .. }
    ));
}

#[test]
fn the_same_message_within_ten_seconds_is_dropped_as_a_repeat() {
    let old = message(
        "OLD",
        DecisionRequired,
        Blocks,
        json!({ "correlationKey": "k", "title": "Same", "summary": "Same summary" }),
    );
    let build = |seconds: i64, extra: serde_json::Value| {
        let mut fields =
            json!({ "correlationKey": "k", "title": "Same", "summary": "Same summary" });
        for (key, value) in extra.as_object().unwrap() {
            fields[key] = value.clone();
        }
        stored_at(
            "NEW",
            NOW + Duration::seconds(seconds),
            DecisionRequired,
            false,
            Blocks,
            fields,
        )
    };
    let items = [item_of(&old, false)];
    let dedupe = CoalesceOutcome::Duplicate {
        of: MessageId::new("OLD"),
        kind: DuplicateKind::Dedupe,
        rescissions: vec![],
    };
    assert_eq!(run(&build(0, json!({})), &items), dedupe);
    assert_eq!(run(&build(9, json!({})), &items), dedupe);
    assert!(
        matches!(
            run(&build(10, json!({})), &items),
            CoalesceOutcome::Superseded { .. }
        ),
        "10 s is outside the window"
    );
    assert!(matches!(
        run(&build(5, json!({ "title": "Different" })), &items),
        CoalesceOutcome::Superseded { .. }
    ));
    assert!(matches!(
        run(&build(5, json!({ "summary": "Different" })), &items),
        CoalesceOutcome::Superseded { .. }
    ));
}

#[test]
fn a_repeat_of_a_different_field_is_not_a_duplicate() {
    let old = stored_at(
        "OLD",
        NOW,
        DecisionRequired,
        false,
        Blocks,
        json!({ "correlationKey": "k" }),
    );
    let urgent = stored_at(
        "NEW",
        NOW + Duration::seconds(1),
        DecisionRequired,
        true,
        Blocks,
        json!({ "correlationKey": "k" }),
    );
    assert!(matches!(
        run(&urgent, &[item_of(&old, false)]),
        CoalesceOutcome::Superseded { pings: true, .. }
    ));
}

#[test]
fn completion_rescinds_waiting_items_without_a_live_waiter_as_handled_in_the_session() {
    let blocking = message(
        "BLOCKING",
        DecisionRequired,
        Blocks,
        json!({ "awaitsResponse": true }),
    );
    let deferrable = message("DEFERRABLE", ActionRequired, Deferrable, json!({}));
    let waiting = message(
        "WAITING",
        DecisionRequired,
        Blocks,
        json!({ "awaitsResponse": true }),
    );
    let non_blocking = message("NONBLOCKING", DecisionRequired, NonBlocking, json!({}));
    let completed = message("DONE", TaskCompleted, NonBlocking, json!({}));
    let items = [
        item_of(&blocking, false),
        item_of(&deferrable, false),
        item_of(&waiting, true),
        item_of(&non_blocking, false),
    ];
    let outcome = run(&completed, &items);
    assert_eq!(
        rescinded_ids(&outcome),
        vec![
            ("BLOCKING".to_owned(), RescindReason::HandledInSession),
            ("DEFERRABLE".to_owned(), RescindReason::HandledInSession),
        ],
        "a live waiter and non_blocking items survive the end of the turn"
    );
}

#[test]
fn only_a_completion_rescinds_by_completion() {
    let blocking = message("BLOCKING", DecisionRequired, Blocks, json!({}));
    for message_type in [DecisionRequired, ActionRequired, Generic] {
        let incoming = message("NEW", message_type, NonBlocking, json!({}));
        assert!(
            rescinded_ids(&run(&incoming, &[item_of(&blocking, false)])).is_empty(),
            "{message_type:?}"
        );
    }
}

#[test]
fn progress_rescinds_idle_and_waiting_items_without_a_live_waiter_as_handled_in_the_session() {
    let conversation = ConversationKey::new("claude-code", "conversation-1");
    let idle = message(
        "IDLE",
        Generic,
        NonBlocking,
        json!({ "correlationKey": IDLE_CORRELATION_KEY }),
    );
    let blocking = message("BLOCKING", DecisionRequired, Blocks, json!({}));
    let deferrable = message("DEFERRABLE", DecisionRequired, Deferrable, json!({}));
    let live = message(
        "LIVE",
        DecisionRequired,
        Blocks,
        json!({ "awaitsResponse": true }),
    );
    let non_blocking = message("NONBLOCKING", DecisionRequired, NonBlocking, json!({}));
    let items = [
        item_of(&idle, false),
        item_of(&blocking, false),
        item_of(&deferrable, false),
        item_of(&live, true),
        item_of(&non_blocking, false),
    ];
    let rescissions = progress_rescissions(&conversation, &items, false);
    let ids: Vec<_> = rescissions
        .iter()
        .map(|r| (r.message_id.as_str(), r.reason))
        .collect();
    assert_eq!(
        ids,
        vec![
            ("IDLE", RescindReason::HandledInSession),
            ("BLOCKING", RescindReason::HandledInSession),
            ("DEFERRABLE", RescindReason::HandledInSession),
        ]
    );
}

#[test]
fn progress_ignores_other_conversations_and_closed_items() {
    let conversation = ConversationKey::new("claude-code", "conversation-1");
    let other = message(
        "OTHER",
        DecisionRequired,
        Blocks,
        json!({ "conversation": { "id": "conversation-2" } }),
    );
    let mut closed = item_of(
        &message("CLOSED", DecisionRequired, Blocks, json!({})),
        false,
    );
    closed.state = LifecycleState::Answered;
    assert!(
        progress_rescissions(&conversation, &[item_of(&other, false), closed], false).is_empty()
    );
}

#[test]
fn topics_and_threads_never_coalesce() {
    // The same key in two conversations sharing a thread keeps separate slots.
    let in_other_conversation = message(
        "OLD",
        DecisionRequired,
        Blocks,
        json!({ "correlationKey": "k", "awaitsResponse": true, "conversation": { "id": "conversation-2" }, "thread": { "key": "phase-3" } }),
    );
    let incoming = message(
        "NEW",
        DecisionRequired,
        Blocks,
        json!({ "correlationKey": "k", "thread": { "key": "phase-3" } }),
    );
    assert_eq!(
        in_other_conversation.thread_id, incoming.thread_id,
        "same thread"
    );
    assert!(matches!(
        run(&incoming, &[item_of(&in_other_conversation, true)]),
        CoalesceOutcome::New { rescissions } if rescissions.is_empty()
    ));
    // A completion in one conversation rescinds nothing in the other.
    let completed = message("DONE", TaskCompleted, NonBlocking, json!({}));
    assert!(rescinded_ids(&run(&completed, &[item_of(&in_other_conversation, false)])).is_empty());
}

#[test]
fn the_rescinds_field_withdraws_the_named_keys_as_part_of_sending() {
    let old = message(
        "OLD",
        DecisionRequired,
        Blocks,
        json!({ "correlationKey": "publish-plan" }),
    );
    let keep = message(
        "KEEP",
        DecisionRequired,
        Blocks,
        json!({ "correlationKey": "other" }),
    );
    let incoming = message(
        "NEW",
        ActionRequired,
        Blocks,
        json!({ "correlationKey": "npm-login" }),
    );
    let request = RescindRequest {
        reason: RescindReason::Superseded,
        correlation_keys: Some(vec!["publish-plan".to_owned()]),
    };
    let outcome = coalesce(&CoalesceInput {
        incoming: &incoming,
        rescinds: Some(&request),
        idempotent_match: None,
        open_items: &[item_of(&old, false), item_of(&keep, false)],
        will_have_live_waiter: false,
        has_phone_channel: false,
    });
    assert_eq!(
        rescinded_ids(&outcome),
        vec![("OLD".to_owned(), RescindReason::Superseded)]
    );
}

#[test]
fn an_item_is_never_rescinded_twice_in_one_outcome() {
    let old = message(
        "OLD",
        DecisionRequired,
        Blocks,
        json!({ "correlationKey": "k" }),
    );
    let incoming = stored_at(
        "NEW",
        NOW + Duration::minutes(1),
        TaskCompleted,
        false,
        NonBlocking,
        json!({ "correlationKey": "k" }),
    );
    let request = RescindRequest {
        reason: RescindReason::Obsolete,
        correlation_keys: Some(vec!["k".to_owned()]),
    };
    let outcome = coalesce(&CoalesceInput {
        incoming: &incoming,
        rescinds: Some(&request),
        idempotent_match: None,
        open_items: &[item_of(&old, false)],
        will_have_live_waiter: false,
        has_phone_channel: false,
    });
    assert_eq!(
        rescinded_ids(&outcome),
        vec![("OLD".to_owned(), RescindReason::Superseded)]
    );
}

#[test]
fn closed_items_are_ignored_by_every_rule() {
    let mut closed = item_of(
        &message(
            "OLD",
            DecisionRequired,
            Blocks,
            json!({ "correlationKey": "k", "awaitsResponse": true }),
        ),
        true,
    );
    closed.state = LifecycleState::Rescinded;
    let incoming = message(
        "NEW",
        DecisionRequired,
        Blocks,
        json!({ "correlationKey": "k" }),
    );
    assert!(matches!(
        run(&incoming, &[closed]),
        CoalesceOutcome::New { .. }
    ));
}

fn keyed_run(
    incoming: &Message,
    items: &[OpenItem],
    will_have_live_waiter: bool,
    rescinds: Option<&RescindRequest>,
) -> CoalesceOutcome {
    coalesce(&CoalesceInput {
        incoming,
        rescinds,
        idempotent_match: None,
        open_items: items,
        will_have_live_waiter,
        has_phone_channel: false,
    })
}

#[test]
fn a_stale_same_key_item_does_not_stop_a_new_blocking_message_from_folding() {
    // A stale blocking item holds the key; a different blocking item has a live waiter.
    let stale = message(
        "STALE",
        DecisionRequired,
        Blocks,
        json!({ "correlationKey": "k" }),
    );
    let live = message(
        "LIVE",
        DecisionRequired,
        Blocks,
        json!({ "correlationKey": "other", "awaitsResponse": true }),
    );
    let incoming = stored_at(
        "NEW",
        NOW + Duration::minutes(1),
        DecisionRequired,
        false,
        Blocks,
        json!({ "correlationKey": "k", "title": "Different" }),
    );
    let items = [item_of(&stale, false), item_of(&live, true)];
    assert_eq!(
        keyed_run(&incoming, &items, false, None),
        CoalesceOutcome::Folded {
            into: MessageId::new("LIVE"),
            rescissions: vec![]
        },
        "§7.3: it is not stored as a new item"
    );
    // A message that will wait still supersedes the stale item in place.
    assert!(matches!(
        keyed_run(&incoming, &items, true, None),
        CoalesceOutcome::Superseded { .. }
    ));
    // Without a live-waiter item the stale item is superseded as before.
    assert!(matches!(
        keyed_run(&incoming, &items[..1], false, None),
        CoalesceOutcome::Superseded { .. }
    ));
    // A non-blocking message is not folded by the blocking-into-blocking rule.
    let calm = stored_at(
        "CALM",
        NOW + Duration::minutes(1),
        DecisionRequired,
        false,
        NonBlocking,
        json!({ "correlationKey": "k" }),
    );
    assert!(matches!(
        keyed_run(&calm, &items, false, None),
        CoalesceOutcome::Superseded { .. }
    ));
}

#[test]
fn a_message_whose_sender_will_not_poll_never_displaces_a_live_waiter() {
    // `awaitsResponse` is declared, but the plan says shouldWait is false (a muted message).
    let live = message(
        "LIVE",
        DecisionRequired,
        Blocks,
        json!({ "correlationKey": "k", "awaitsResponse": true, "title": "Old" }),
    );
    let incoming = stored_at(
        "NEW",
        NOW + Duration::minutes(1),
        DecisionRequired,
        false,
        Blocks,
        json!({ "correlationKey": "k", "awaitsResponse": true, "title": "New" }),
    );
    let items = [item_of(&live, true)];
    assert!(matches!(
        keyed_run(&incoming, &items, true, None),
        CoalesceOutcome::Superseded { .. }
    ));
    assert_eq!(
        keyed_run(&incoming, &items, false, None),
        CoalesceOutcome::Folded {
            into: MessageId::new("LIVE"),
            rescissions: vec![]
        }
    );
}

#[test]
fn the_hubs_waiter_flag_is_the_plans_should_wait() {
    use gentbox_core::policy::{Policy, decide, expects_live_waiter};
    use support::calm_context;
    let awaiting_message = support::awaiting(DecisionRequired, false, Blocks);
    let mut muted_context = calm_context();
    muted_context.is_muted = true;
    for context in [calm_context(), muted_context] {
        assert_eq!(
            expects_live_waiter(&awaiting_message, &context),
            decide(&awaiting_message, &context, &Policy::default()).should_wait
        );
    }
    let mut urgent = awaiting_message.clone();
    urgent.urgent = true;
    let mut muted_context = calm_context();
    muted_context.is_muted = true;
    assert!(
        expects_live_waiter(&urgent, &muted_context),
        "urgent breaks through a mute"
    );
    let mut rule_muted = awaiting_message.clone();
    rule_muted.muted_by_rule = Some("Mute".to_owned());
    assert!(!expects_live_waiter(&rule_muted, &calm_context()));
    let not_awaiting = message("NA", DecisionRequired, Blocks, json!({}));
    assert!(!expects_live_waiter(&not_awaiting, &calm_context()));
    let completed = support::awaiting(TaskCompleted, false, NonBlocking);
    assert!(!expects_live_waiter(&completed, &calm_context()));
}

#[test]
fn a_folded_or_deduped_message_still_applies_its_rescinds() {
    let target = message(
        "TARGET",
        DecisionRequired,
        Blocks,
        json!({ "correlationKey": "publish-plan" }),
    );
    let live = message(
        "LIVE",
        DecisionRequired,
        Blocks,
        json!({ "correlationKey": "k", "awaitsResponse": true }),
    );
    let request = RescindRequest {
        reason: RescindReason::Superseded,
        correlation_keys: Some(vec!["publish-plan".to_owned(), "k".to_owned()]),
    };

    let folded = message(
        "NEW",
        DecisionRequired,
        Blocks,
        json!({ "correlationKey": "k" }),
    );
    let items = [item_of(&target, false), item_of(&live, true)];
    let CoalesceOutcome::Folded { into, rescissions } =
        keyed_run(&folded, &items, false, Some(&request))
    else {
        panic!("expected a fold")
    };
    assert_eq!(into.as_str(), "LIVE");
    let ids: Vec<_> = rescissions.iter().map(|r| r.message_id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["TARGET"],
        "the absorbing item is never rescinded by the message it absorbed"
    );

    let old = message(
        "OLD",
        DecisionRequired,
        Blocks,
        json!({ "correlationKey": "same", "title": "Same" }),
    );
    let repeat = stored_at(
        "REPEAT",
        NOW + Duration::seconds(2),
        DecisionRequired,
        false,
        Blocks,
        json!({ "correlationKey": "same", "title": "Same" }),
    );
    let request = RescindRequest {
        reason: RescindReason::Obsolete,
        correlation_keys: Some(vec!["publish-plan".to_owned()]),
    };
    let items = [item_of(&old, false), item_of(&target, false)];
    let CoalesceOutcome::Duplicate {
        kind, rescissions, ..
    } = keyed_run(&repeat, &items, false, Some(&request))
    else {
        panic!("expected a dedupe")
    };
    assert_eq!(kind, DuplicateKind::Dedupe);
    assert_eq!(rescissions.len(), 1);
    assert_eq!(rescissions[0].message_id.as_str(), "TARGET");
}

#[test]
fn an_idempotent_retry_changes_nothing_not_even_through_rescinds() {
    let target = message(
        "TARGET",
        DecisionRequired,
        Blocks,
        json!({ "correlationKey": "publish-plan" }),
    );
    let incoming = message("NEW", DecisionRequired, Blocks, json!({}));
    let existing = MessageId::new("STORED");
    let request = RescindRequest {
        reason: RescindReason::Obsolete,
        correlation_keys: Some(vec!["publish-plan".to_owned()]),
    };
    let outcome = coalesce(&CoalesceInput {
        incoming: &incoming,
        rescinds: Some(&request),
        idempotent_match: Some(&existing),
        open_items: &[item_of(&target, false)],
        will_have_live_waiter: false,
        has_phone_channel: false,
    });
    assert_eq!(
        outcome,
        CoalesceOutcome::Duplicate {
            of: existing,
            kind: DuplicateKind::Idempotent,
            rescissions: vec![]
        }
    );
}
