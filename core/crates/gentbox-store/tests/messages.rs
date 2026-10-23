//! Messages: round trips, lookups, listings, updates, supersede chains,
//! conversations and the delivery log.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use gentbox_core::classify::FieldSource;
use gentbox_core::ids::ConversationKey;
use gentbox_core::lifecycle::{LifecycleState, RecordedAnswer, ResolutionReason};
use gentbox_core::message::{
    Action, ActionKind, Blocking, Conversation, FocusTarget, Message, MessageType, TerminalTarget,
};
use gentbox_core::policy::PolicyKey;
use gentbox_core::rescind::RescindReason;
use gentbox_core::response::AnsweredVia;
use gentbox_store::{
    Conflict, DeliveryEvent, DeliveryRecord, EntityKind, MessageFilter, Store, StoreError,
};
use serde_json::json;
use support::{
    NOW, classified, closed, memory_store, message, message_id, plain, seed_group, thread,
    thread_id, topic, topic_id,
};
use time::macros::datetime;
use time::{Duration, OffsetDateTime};

fn store_with_group() -> Store {
    let store = memory_store();
    seed_group(&store.repository(), "one");
    store
}

fn round_trip(store: &Store, message: &Message) -> Message {
    let repository = store.repository();
    repository.insert_message(message, NOW).unwrap();
    repository.message(&message.id).unwrap()
}

#[test]
fn every_type_urgency_and_blocking_combination_round_trips() {
    let store = store_with_group();
    let combinations = PolicyKey::all_valid();
    assert_eq!(combinations.len(), 15);
    for (index, key) in combinations.iter().enumerate() {
        let message = classified(
            &format!("combo-{index}"),
            "one",
            "one",
            key.message_type,
            key.urgent,
            key.blocking,
        );
        assert_eq!(round_trip(&store, &message), message);
    }
}

#[test]
fn a_message_with_every_optional_field_round_trips() {
    let store = store_with_group();
    let mut message = message(
        "rich",
        "one",
        "one",
        json!({
            "idempotencyKey": "retry-1",
            "source": {
                "agent": "claude-code",
                "displayName": "Claude Code",
                "version": "2.1.0",
                "host": "mac-studio",
            },
            "conversation": {
                "id": "conversation-1",
                "title": "Ship the store",
                "cwd": "/Users/me/Developer/gentbox",
                "focus": {
                    "url": "claude-cli://open?session=abc",
                    "bundleId": "com.apple.Terminal",
                    "terminal": { "tty": "/dev/ttys004", "tmuxPane": "%3", "pid": 4242 },
                },
            },
            "type": "action_required",
            "urgent": true,
            "blocking": "deferrable",
            "event": "claude-code.stop_failure",
            "title": "Claude Code hit an API error",
            "summary": "The turn ended on an API error.",
            "body": "```\nHTTP 529\n```",
            "steps": [
                { "text": "Open the session" },
                {
                    "text": "Read the error",
                    "detail": "Look at `last_error` in the log.",
                    "link": { "url": "https://status.example.com", "label": "Status" },
                },
            ],
            "actions": [
                { "id": "done", "label": "Done", "kind": "done" },
                { "id": "later", "label": "Later", "kind": "defer" },
                { "id": "reply", "label": "Reply", "kind": "reply", "placeholder": "What next?" },
                { "id": "nuke", "label": "Delete all", "kind": "choice", "value": "nuke", "destructive": true },
            ],
            "awaitsResponse": true,
            "correlationKey": "stop-failure",
            "sensitive": true,
            "createdAt": "2026-10-24T11:59:00Z",
            "expiresAt": "2026-10-25T11:59:00Z",
            "metadata": { "cost": 12, "dry": false, "model": "opus" },
        }),
    );
    message.steps_completed = vec![0];
    message.repeat_count = 3;
    message.muted_by_rule = Some("Mute auth success".to_owned());
    message.lifecycle.deferred_at = Some(NOW + Duration::minutes(5));
    message.field_sources.message_type = FieldSource::UserRule("Quiet the docs bot".to_owned());
    message.field_sources.urgent = FieldSource::BuiltInRule("permission prompts".to_owned());
    message.field_sources.blocking = FieldSource::Normalized;

    let stored = round_trip(&store, &message);
    assert_eq!(stored, message);
    assert_eq!(
        stored.conversation.focus.as_ref().unwrap().terminal,
        Some(TerminalTarget {
            tty: Some("/dev/ttys004".to_owned()),
            tmux_pane: Some("%3".to_owned()),
            pid: Some(4242),
        })
    );
}

#[test]
fn every_action_kind_round_trips_in_order() {
    let store = store_with_group();
    let kinds = [
        ActionKind::Approve,
        ActionKind::Deny,
        ActionKind::Choice,
        ActionKind::Reply,
        ActionKind::Done,
        ActionKind::Defer,
        ActionKind::Open,
        ActionKind::Snooze,
    ];
    let mut message = plain("actions", "one", "one");
    message.actions = kinds
        .iter()
        .rev()
        .map(|kind| Action {
            id: format!("{kind:?}"),
            label: format!("{kind:?} it"),
            kind: *kind,
            value: Some("15".to_owned()),
            placeholder: Some("hint".to_owned()),
            destructive: *kind == ActionKind::Deny,
        })
        .collect();
    assert_eq!(round_trip(&store, &message), message);
}

#[test]
fn every_closed_state_resolution_and_answer_round_trips() {
    let store = store_with_group();
    let mut cases: Vec<Message> = Vec::new();
    for (index, via) in [
        AnsweredVia::Mac,
        AnsweredVia::Phone,
        AnsweredVia::Watch,
        AnsweredVia::Cli,
    ]
    .into_iter()
    .enumerate()
    {
        let mut answered = closed(
            plain(&format!("answered-{index}"), "one", "one"),
            LifecycleState::Answered,
        );
        answered.lifecycle.answer = Some(RecordedAnswer {
            action_id: "approve".to_owned(),
            action_kind: ActionKind::Approve,
            value: Some("yes".to_owned()),
            answered_via: Some(via),
            answered_at: NOW + Duration::seconds(30),
        });
        cases.push(answered);
    }
    let mut done_without_via = closed(plain("done", "one", "one"), LifecycleState::Answered);
    done_without_via.lifecycle.answer = Some(RecordedAnswer {
        action_id: "done".to_owned(),
        action_kind: ActionKind::Done,
        value: None,
        answered_via: None,
        answered_at: NOW,
    });
    cases.push(done_without_via);
    cases.push(closed(
        plain("dismissed", "one", "one"),
        LifecycleState::Dismissed,
    ));
    cases.push(closed(
        plain("expired", "one", "one"),
        LifecycleState::Expired,
    ));
    for reason in RescindReason::ALL {
        let mut rescinded = closed(
            plain(&format!("rescinded-{reason:?}"), "one", "one"),
            LifecycleState::Rescinded,
        );
        rescinded.lifecycle.resolution = Some(ResolutionReason::Rescinded(reason));
        cases.push(rescinded);
    }
    let mut snoozed = plain("snoozed", "one", "one");
    snoozed.lifecycle.state = LifecycleState::Acknowledged;
    snoozed.lifecycle.snoozed_until = Some(NOW + Duration::minutes(15));
    cases.push(snoozed);
    for case in &cases {
        assert_eq!(&round_trip(&store, case), case, "{:?}", case.id);
    }
}

#[test]
fn timestamps_round_trip_to_the_nanosecond() {
    let store = store_with_group();
    let mut message = plain("precise", "one", "one");
    message.received_at = datetime!(2026-10-24 12:00:00.123456789 UTC);
    message.expires_at = datetime!(2026-10-24 14:00:00.000000001 +02:00);
    message.created_at = Some(datetime!(2026-10-24 11:59:59.999999999 -05:00));
    assert_eq!(round_trip(&store, &message), message);
}

#[test]
fn a_timestamp_the_column_cannot_hold_is_an_error_not_a_panic() {
    let store = store_with_group();
    let mut message = plain("far", "one", "one");
    message.expires_at = datetime!(2500-01-01 0:00 UTC);
    let error = store
        .repository()
        .insert_message(&message, NOW)
        .unwrap_err();
    assert!(matches!(error, StoreError::TimestampOutOfRange));
    assert!(
        store
            .repository()
            .find_message(&message.id)
            .unwrap()
            .is_none()
    );
}

#[test]
fn a_duplicate_message_id_is_a_conflict() {
    let store = store_with_group();
    let message = plain("dup", "one", "one");
    store.repository().insert_message(&message, NOW).unwrap();
    let error = store
        .repository()
        .insert_message(&message, NOW)
        .unwrap_err();
    assert!(matches!(error, StoreError::Conflict(Conflict::MessageId(id)) if id == message.id));
}

#[test]
fn an_idempotency_key_is_unique_per_agent() {
    let store = store_with_group();
    let first = message("first", "one", "one", json!({ "idempotencyKey": "retry" }));
    store.repository().insert_message(&first, NOW).unwrap();

    let second = message("second", "one", "one", json!({ "idempotencyKey": "retry" }));
    let error = store.repository().insert_message(&second, NOW).unwrap_err();
    assert!(matches!(
        error,
        StoreError::Conflict(Conflict::IdempotencyKey { ref agent, ref key })
            if agent == "claude-code" && key == "retry"
    ));

    let other_agent = message(
        "third",
        "one",
        "one",
        json!({ "idempotencyKey": "retry", "source": { "agent": "codex" } }),
    );
    store
        .repository()
        .insert_message(&other_agent, NOW)
        .unwrap();

    let found = store
        .repository()
        .find_message_by_idempotency_key("claude-code", "retry")
        .unwrap();
    assert_eq!(found.map(|message| message.id), Some(first.id));
    assert!(
        store
            .repository()
            .find_message_by_idempotency_key("claude-code", "other")
            .unwrap()
            .is_none()
    );
}

#[test]
fn messages_without_an_idempotency_key_never_collide() {
    let store = store_with_group();
    for name in ["a", "b", "c"] {
        store
            .repository()
            .insert_message(&plain(name, "one", "one"), NOW)
            .unwrap();
    }
}

#[test]
fn a_missing_message_is_not_found() {
    let store = store_with_group();
    let error = store
        .repository()
        .message(&message_id("ghost"))
        .unwrap_err();
    assert!(matches!(
        error,
        StoreError::NotFound { entity: EntityKind::Message, ref id } if id == "message-ghost"
    ));
    assert!(
        store
            .repository()
            .find_message(&message_id("ghost"))
            .unwrap()
            .is_none()
    );
    let error = store
        .repository()
        .update_message(&plain("ghost", "one", "one"), NOW)
        .unwrap_err();
    assert!(matches!(
        error,
        StoreError::NotFound {
            entity: EntityKind::Message,
            ..
        }
    ));
}

#[test]
fn a_message_in_an_unknown_thread_is_not_found_and_leaves_nothing_behind() {
    let store = memory_store();
    let message = plain("orphan", "one", "one");
    let error = store
        .repository()
        .insert_message(&message, NOW)
        .unwrap_err();
    assert!(matches!(
        error,
        StoreError::NotFound {
            entity: EntityKind::Thread,
            ..
        }
    ));
    let error = store
        .repository()
        .conversation(&message.conversation_key())
        .unwrap_err();
    assert!(matches!(
        error,
        StoreError::NotFound {
            entity: EntityKind::Conversation,
            ..
        }
    ));
}

#[test]
#[should_panic(expected = "names topic")]
fn a_message_whose_topic_is_not_its_threads_topic_is_a_broken_invariant() {
    let store = store_with_group();
    store.repository().insert_topic(&topic("two")).unwrap();
    let message = plain("skewed", "two", "one");
    let _ = store.repository().insert_message(&message, NOW);
}

#[test]
fn update_saves_the_mutable_parts_and_replaces_the_actions() {
    let store = store_with_group();
    let mut message = message(
        "mutable",
        "one",
        "one",
        json!({ "type": "decision_required", "blocking": "deferrable", "actions": [
			{ "id": "ok", "label": "OK", "kind": "approve" },
			{ "id": "later", "label": "Later", "kind": "defer" },
		] }),
    );
    store.repository().insert_message(&message, NOW).unwrap();

    message.blocking = Blocking::NonBlocking;
    message.lifecycle.deferred_at = Some(NOW + Duration::minutes(1));
    message.title = "Allow the ingest tests?".to_owned();
    message.repeat_count = 2;
    message.steps_completed = vec![];
    message.actions.truncate(1);
    message.lifecycle.state = LifecycleState::Acknowledged;
    store
        .repository()
        .update_message(&message, NOW + Duration::minutes(1))
        .unwrap();
    assert_eq!(store.repository().message(&message.id).unwrap(), message);

    message.lifecycle.state = LifecycleState::Answered;
    message.lifecycle.resolution = Some(ResolutionReason::Answered);
    message.lifecycle.answer = Some(RecordedAnswer {
        action_id: "ok".to_owned(),
        action_kind: ActionKind::Approve,
        value: None,
        answered_via: Some(AnsweredVia::Mac),
        answered_at: NOW + Duration::minutes(2),
    });
    store
        .repository()
        .update_message(&message, NOW + Duration::minutes(2))
        .unwrap();
    assert_eq!(store.repository().message(&message.id).unwrap(), message);
}

#[test]
fn closed_at_is_set_once_when_the_message_closes() {
    let store = store_with_group();
    let message = plain("closing", "one", "one");
    store.repository().insert_message(&message, NOW).unwrap();
    assert_eq!(store.repository().closed_at(&message.id).unwrap(), None);

    let answered = closed(message, LifecycleState::Answered);
    let closing_time = NOW + Duration::hours(1);
    store
        .repository()
        .update_message(&answered, closing_time)
        .unwrap();
    assert_eq!(
        store.repository().closed_at(&answered.id).unwrap(),
        Some(closing_time)
    );

    store
        .repository()
        .update_message(&answered, closing_time + Duration::days(1))
        .unwrap();
    assert_eq!(
        store.repository().closed_at(&answered.id).unwrap(),
        Some(closing_time)
    );

    let born_closed = closed(plain("born-closed", "one", "one"), LifecycleState::Expired);
    store
        .repository()
        .insert_message(&born_closed, closing_time)
        .unwrap();
    assert_eq!(
        store.repository().closed_at(&born_closed.id).unwrap(),
        Some(closing_time)
    );
}

#[test]
fn an_unknown_message_has_no_closing_time() {
    let store = store_with_group();
    let error = store
        .repository()
        .closed_at(&message_id("ghost"))
        .unwrap_err();
    assert!(matches!(
        error,
        StoreError::NotFound {
            entity: EntityKind::Message,
            ..
        }
    ));
}

// --- listing -------------------------------------------------------------

fn store_with_varied_messages() -> Store {
    let store = store_with_group();
    let repository = store.repository();
    repository.insert_topic(&topic("two")).unwrap();
    repository.insert_thread(&thread("two", "two")).unwrap();
    let rows = [
        (
            "a",
            "one",
            "one",
            "conversation-1",
            MessageType::DecisionRequired,
            false,
            Blocking::Blocking,
        ),
        (
            "b",
            "one",
            "one",
            "conversation-1",
            MessageType::DecisionRequired,
            true,
            Blocking::Deferrable,
        ),
        (
            "c",
            "one",
            "one",
            "conversation-2",
            MessageType::ActionRequired,
            false,
            Blocking::NonBlocking,
        ),
        (
            "d",
            "two",
            "two",
            "conversation-2",
            MessageType::TaskCompleted,
            true,
            Blocking::NonBlocking,
        ),
        (
            "e",
            "two",
            "two",
            "conversation-1",
            MessageType::Generic,
            false,
            Blocking::NonBlocking,
        ),
    ];
    for (index, (name, topic, thread, conversation, message_type, urgent, blocking)) in
        rows.into_iter().enumerate()
    {
        let mut message = classified(name, topic, thread, message_type, urgent, blocking);
        message.conversation.id = conversation.to_owned();
        message.received_at = NOW + Duration::seconds(i64::try_from(index).unwrap());
        repository.insert_message(&message, NOW).unwrap();
    }
    store
}

fn names(messages: &[Message]) -> Vec<String> {
    messages
        .iter()
        .map(|message| {
            message
                .id
                .as_str()
                .trim_start_matches("message-")
                .to_owned()
        })
        .collect()
}

#[test]
fn the_default_listing_is_every_live_message_newest_first() {
    let store = store_with_varied_messages();
    let listed = store
        .repository()
        .list_messages(&MessageFilter::default())
        .unwrap();
    assert_eq!(names(&listed), ["e", "d", "c", "b", "a"]);
}

#[test]
fn open_messages_are_listed_by_conversation() {
    let store = store_with_varied_messages();
    let filter = MessageFilter {
        conversation: Some(ConversationKey::new("claude-code", "conversation-2")),
        ..MessageFilter::default()
    };
    assert_eq!(
        names(&store.repository().list_messages(&filter).unwrap()),
        ["d", "c"]
    );
}

#[test]
fn open_messages_are_listed_by_type() {
    let store = store_with_varied_messages();
    for (message_type, expected) in [
        (MessageType::DecisionRequired, vec!["b", "a"]),
        (MessageType::ActionRequired, vec!["c"]),
        (MessageType::TaskCompleted, vec!["d"]),
        (MessageType::Generic, vec!["e"]),
    ] {
        let filter = MessageFilter {
            message_type: Some(message_type),
            ..MessageFilter::default()
        };
        assert_eq!(
            names(&store.repository().list_messages(&filter).unwrap()),
            expected,
            "{message_type:?}"
        );
    }
}

#[test]
fn open_messages_are_listed_by_urgency() {
    let store = store_with_varied_messages();
    let urgent = MessageFilter {
        urgent: Some(true),
        ..MessageFilter::default()
    };
    assert_eq!(
        names(&store.repository().list_messages(&urgent).unwrap()),
        ["d", "b"]
    );
    let calm = MessageFilter {
        urgent: Some(false),
        ..MessageFilter::default()
    };
    assert_eq!(
        names(&store.repository().list_messages(&calm).unwrap()),
        ["e", "c", "a"]
    );
}

#[test]
fn open_messages_are_listed_by_blocking_level() {
    let store = store_with_varied_messages();
    for (blocking, expected) in [
        (Blocking::Blocking, vec!["a"]),
        (Blocking::Deferrable, vec!["b"]),
        (Blocking::NonBlocking, vec!["e", "d", "c"]),
    ] {
        let filter = MessageFilter {
            blocking: Some(blocking),
            ..MessageFilter::default()
        };
        assert_eq!(
            names(&store.repository().list_messages(&filter).unwrap()),
            expected,
            "{blocking:?}"
        );
    }
}

#[test]
fn filters_combine_and_scope_to_topic_and_thread() {
    let store = store_with_varied_messages();
    let filter = MessageFilter {
        conversation: Some(ConversationKey::new("claude-code", "conversation-1")),
        topic_id: Some(topic_id("one")),
        urgent: Some(true),
        ..MessageFilter::default()
    };
    assert_eq!(
        names(&store.repository().list_messages(&filter).unwrap()),
        ["b"]
    );
    let by_thread = MessageFilter {
        thread_id: Some(thread_id("two")),
        ..MessageFilter::default()
    };
    assert_eq!(
        names(&store.repository().list_messages(&by_thread).unwrap()),
        ["e", "d"]
    );
    let by_topic = MessageFilter {
        topic_id: Some(topic_id("two")),
        ..MessageFilter::default()
    };
    assert_eq!(
        names(&store.repository().list_messages(&by_topic).unwrap()),
        ["e", "d"]
    );
}

#[test]
fn closed_messages_leave_the_open_listing_but_stay_listable_by_state() {
    let store = store_with_varied_messages();
    let repository = store.repository();
    let answered = closed(
        repository.message(&message_id("a")).unwrap(),
        LifecycleState::Answered,
    );
    repository.update_message(&answered, NOW).unwrap();

    assert_eq!(
        names(&repository.list_messages(&MessageFilter::default()).unwrap()),
        ["e", "d", "c", "b"]
    );
    let only_answered = MessageFilter {
        states: Some(vec![LifecycleState::Answered]),
        ..MessageFilter::default()
    };
    assert_eq!(
        names(&repository.list_messages(&only_answered).unwrap()),
        ["a"]
    );
    let everything = MessageFilter {
        states: Some(LifecycleState::ALL.to_vec()),
        ..MessageFilter::default()
    };
    assert_eq!(repository.list_messages(&everything).unwrap().len(), 5);
}

#[test]
fn open_only_excludes_acknowledged_and_snoozed_messages_hide_until_they_return() {
    let store = store_with_varied_messages();
    let repository = store.repository();
    let mut snoozed = repository.message(&message_id("b")).unwrap();
    snoozed.lifecycle.state = LifecycleState::Acknowledged;
    snoozed.lifecycle.snoozed_until = Some(NOW + Duration::minutes(15));
    repository.update_message(&snoozed, NOW).unwrap();

    let only_open = MessageFilter {
        states: Some(vec![LifecycleState::Open]),
        ..MessageFilter::default()
    };
    assert_eq!(
        names(&repository.list_messages(&only_open).unwrap()),
        ["d", "c", "a"]
    );

    let hiding = |at| MessageFilter {
        hide_snoozed_at: Some(at),
        ..MessageFilter::default()
    };
    assert_eq!(
        names(&repository.list_messages(&hiding(NOW)).unwrap()),
        ["e", "d", "c", "a"]
    );
    assert_eq!(
        names(
            &repository
                .list_messages(&hiding(NOW + Duration::minutes(15)))
                .unwrap()
        )
        .len(),
        5
    );
}

#[test]
fn a_listing_can_be_limited() {
    let store = store_with_varied_messages();
    let filter = MessageFilter {
        limit: Some(2),
        ..MessageFilter::default()
    };
    assert_eq!(
        names(&store.repository().list_messages(&filter).unwrap()),
        ["e", "d"]
    );
}

#[test]
fn listed_messages_carry_their_actions_and_conversation() {
    let store = store_with_group();
    let mut message = message(
        "listed",
        "one",
        "one",
        json!({
            "conversation": { "id": "conversation-1", "title": "Ship the store" },
            "actions": [{ "id": "ok", "label": "OK", "kind": "approve" }],
        }),
    );
    message.lifecycle.state = LifecycleState::Open;
    store.repository().insert_message(&message, NOW).unwrap();
    let listed = store
        .repository()
        .list_messages(&MessageFilter::default())
        .unwrap();
    assert_eq!(listed, vec![message]);
}

// --- rescinded and superseded ----------------------------------------------

#[test]
fn rescinded_messages_are_kept_with_their_reason() {
    let store = store_with_group();
    let repository = store.repository();
    for reason in RescindReason::ALL {
        let mut message = plain(&format!("{reason:?}"), "one", "one");
        repository.insert_message(&message, NOW).unwrap();
        message.lifecycle.state = LifecycleState::Rescinded;
        message.lifecycle.resolution = Some(ResolutionReason::Rescinded(reason));
        repository.update_message(&message, NOW).unwrap();
    }
    let filter = MessageFilter {
        states: Some(vec![LifecycleState::Rescinded]),
        ..MessageFilter::default()
    };
    let kept = repository.list_messages(&filter).unwrap();
    let mut reasons: Vec<_> = kept
        .iter()
        .map(|message| message.lifecycle.resolution)
        .collect();
    reasons.sort_by_key(|reason| format!("{reason:?}"));
    assert_eq!(
        reasons,
        [
            Some(ResolutionReason::Rescinded(RescindReason::Obsolete)),
            Some(ResolutionReason::Rescinded(
                RescindReason::ResolvedElsewhere
            )),
            Some(ResolutionReason::Rescinded(RescindReason::Superseded)),
        ]
    );
    assert!(
        repository
            .list_messages(&MessageFilter::default())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn the_supersede_chain_is_recorded_and_followed() {
    let store = store_with_group();
    let repository = store.repository();
    for name in ["v1", "v2", "v3"] {
        repository
            .insert_message(&plain(name, "one", "one"), NOW)
            .unwrap();
    }
    repository
        .record_supersession(&message_id("v1"), &message_id("v2"))
        .unwrap();
    repository
        .record_supersession(&message_id("v2"), &message_id("v3"))
        .unwrap();

    assert_eq!(
        repository.superseded_by(&message_id("v1")).unwrap(),
        Some(message_id("v2"))
    );
    assert_eq!(repository.superseded_by(&message_id("v3")).unwrap(), None);
    assert_eq!(
        repository.supersede_chain(&message_id("v1")).unwrap(),
        [message_id("v2"), message_id("v3")]
    );
    assert_eq!(repository.supersede_chain(&message_id("v3")).unwrap(), []);
}

#[test]
fn superseding_with_an_unknown_message_is_not_found() {
    let store = store_with_group();
    let repository = store.repository();
    repository
        .insert_message(&plain("v1", "one", "one"), NOW)
        .unwrap();
    for (replaced, successor) in [("v1", "ghost"), ("ghost", "v1")] {
        let error = repository
            .record_supersession(&message_id(replaced), &message_id(successor))
            .unwrap_err();
        assert!(matches!(
            error,
            StoreError::NotFound {
                entity: EntityKind::Message,
                ..
            }
        ));
    }
    assert!(matches!(
        repository.superseded_by(&message_id("ghost")).unwrap_err(),
        StoreError::NotFound { .. }
    ));
}

#[test]
#[should_panic(expected = "cannot supersede itself")]
fn a_message_cannot_supersede_itself() {
    let store = store_with_group();
    store
        .repository()
        .insert_message(&plain("v1", "one", "one"), NOW)
        .unwrap();
    let _ = store
        .repository()
        .record_supersession(&message_id("v1"), &message_id("v1"));
}

// --- due messages ------------------------------------------------------------

#[test]
fn due_expiries_and_snoozes_are_listed_oldest_first() {
    let store = store_with_group();
    let repository = store.repository();
    let mut soon = plain("soon", "one", "one");
    soon.expires_at = NOW + Duration::hours(1);
    let mut later = plain("later", "one", "one");
    later.expires_at = NOW + Duration::hours(2);
    let mut answered = closed(plain("answered", "one", "one"), LifecycleState::Answered);
    answered.expires_at = NOW + Duration::minutes(1);
    let mut snoozed = plain("snoozed", "one", "one");
    snoozed.expires_at = NOW + Duration::days(1);
    snoozed.lifecycle.state = LifecycleState::Acknowledged;
    snoozed.lifecycle.snoozed_until = Some(NOW + Duration::minutes(10));
    for message in [&soon, &later, &answered, &snoozed] {
        repository.insert_message(message, NOW).unwrap();
    }

    assert_eq!(
        repository
            .list_live_messages_expired_by(NOW + Duration::hours(3))
            .unwrap(),
        [soon.id.clone(), later.id.clone()]
    );
    assert_eq!(
        repository
            .list_live_messages_expired_by(NOW + Duration::hours(1))
            .unwrap(),
        [soon.id.clone()]
    );
    assert_eq!(repository.list_live_messages_expired_by(NOW).unwrap(), []);
    assert_eq!(repository.list_snoozes_ended_by(NOW).unwrap(), []);
    assert_eq!(
        repository
            .list_snoozes_ended_by(NOW + Duration::minutes(10))
            .unwrap(),
        [snoozed.id]
    );
}

// --- conversations ---------------------------------------------------------------

#[test]
fn a_conversation_is_registered_and_renamed_without_losing_other_fields() {
    let store = store_with_group();
    let repository = store.repository();
    let key = ConversationKey::new("claude-code", "conversation-9");
    let first = Conversation {
        id: "conversation-9".to_owned(),
        title: Some("First title".to_owned()),
        cwd: Some("/Users/me/Developer/gentbox".to_owned()),
        focus: Some(FocusTarget {
            url: None,
            bundle_id: Some("com.apple.Terminal".to_owned()),
            terminal: None,
        }),
    };
    repository.upsert_conversation(&key, &first, NOW).unwrap();
    assert_eq!(repository.conversation(&key).unwrap(), first);

    let renamed = Conversation {
        title: Some("Renamed".to_owned()),
        cwd: None,
        focus: None,
        ..first.clone()
    };
    repository
        .upsert_conversation(&key, &renamed, NOW + Duration::minutes(1))
        .unwrap();
    assert_eq!(
        repository.conversation(&key).unwrap(),
        Conversation {
            title: Some("Renamed".to_owned()),
            ..first
        }
    );
}

#[test]
fn a_rename_shows_on_the_conversations_messages() {
    let store = store_with_group();
    let repository = store.repository();
    let message = plain("renamed", "one", "one");
    repository.insert_message(&message, NOW).unwrap();
    let renamed = Conversation {
        title: Some("Now with a title".to_owned()),
        ..message.conversation.clone()
    };
    repository
        .upsert_conversation(&message.conversation_key(), &renamed, NOW)
        .unwrap();
    assert_eq!(
        repository
            .message(&message.id)
            .unwrap()
            .conversation
            .title
            .as_deref(),
        Some("Now with a title")
    );
}

#[test]
fn an_unknown_conversation_is_not_found() {
    let store = store_with_group();
    let error = store
        .repository()
        .conversation(&ConversationKey::new("claude-code", "ghost"))
        .unwrap_err();
    assert!(matches!(
        error,
        StoreError::NotFound {
            entity: EntityKind::Conversation,
            ..
        }
    ));
}

// --- delivery log ---------------------------------------------------------------

#[test]
fn the_delivery_log_keeps_its_entries_in_order() {
    let store = store_with_group();
    let repository = store.repository();
    let message = plain("logged", "one", "one");
    repository.insert_message(&message, NOW).unwrap();
    let entries = [
        (
            DeliveryEvent::Normalized,
            None,
            Some("urgent changed from true to false"),
        ),
        (DeliveryEvent::Delivered, Some("popup"), None),
        (
            DeliveryEvent::Repinged,
            Some("popup"),
            Some("after 10 minutes"),
        ),
        (DeliveryEvent::Warned, None, Some("title rewritten")),
        (DeliveryEvent::Withdrawn, Some("popup"), Some("obsolete")),
    ];
    let records: Vec<DeliveryRecord> = entries
        .into_iter()
        .enumerate()
        .map(|(index, (event, surface, detail))| DeliveryRecord {
            message_id: message.id.clone(),
            recorded_at: NOW + Duration::seconds(i64::try_from(index).unwrap()),
            event,
            surface: surface.map(str::to_owned),
            detail: detail.map(str::to_owned),
        })
        .collect();
    for record in &records {
        repository.append_delivery(record).unwrap();
    }
    assert_eq!(repository.deliveries(&message.id).unwrap(), records);
}

#[test]
fn the_delivery_log_of_an_unknown_message_is_not_found() {
    let store = store_with_group();
    let record = DeliveryRecord {
        message_id: message_id("ghost"),
        recorded_at: NOW,
        event: DeliveryEvent::Delivered,
        surface: None,
        detail: None,
    };
    assert!(matches!(
        store.repository().append_delivery(&record).unwrap_err(),
        StoreError::NotFound {
            entity: EntityKind::Message,
            ..
        }
    ));
    assert!(matches!(
        store
            .repository()
            .deliveries(&message_id("ghost"))
            .unwrap_err(),
        StoreError::NotFound { .. }
    ));
}

// --- transactions -----------------------------------------------------------------

#[test]
fn a_transaction_commits_all_or_nothing() {
    let mut store = store_with_group();
    let result: Result<(), StoreError> = store.transaction(|repository| {
        repository.insert_message(&plain("kept-not", "one", "one"), NOW)?;
        repository.insert_message(&plain("kept-not", "one", "one"), NOW)?;
        Ok(())
    });
    assert!(matches!(
        result,
        Err(StoreError::Conflict(Conflict::MessageId(_)))
    ));
    assert!(
        store
            .repository()
            .find_message(&message_id("kept-not"))
            .unwrap()
            .is_none()
    );

    store
        .transaction(|repository| {
            repository.insert_message(&plain("one", "one", "one"), NOW)?;
            repository.insert_message(&plain("two", "one", "one"), NOW)
        })
        .unwrap();
    assert_eq!(
        store
            .repository()
            .list_messages(&MessageFilter::default())
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn a_unix_timestamp_survives_the_round_trip() {
    let store = store_with_group();
    let mut message = plain("zone", "one", "one");
    message.received_at = OffsetDateTime::from_unix_timestamp(1_800_000_000).unwrap();
    let stored = round_trip(&store, &message);
    assert_eq!(stored.received_at.unix_timestamp(), 1_800_000_000);
}
