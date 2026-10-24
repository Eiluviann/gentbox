//! Answers through the hub handle (as the UI gives them), Defer without
//! redelivery, ownership of rescinds and moves, re-pings, snooze and expiry.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::time::Duration as StdDuration;

use gentbox_core::message::{ActionKind, Blocking};
use gentbox_core::rescind::RescindReason;
use gentbox_core::response::{AnsweredVia, ResponseStatus};
use gentbox_hub::{ActionRequest, HubError, HubEvent, MessageThreadMove, Requester};
use serde_json::json;
use support::{TestHub, blocking_prompt, count, deferrable_question, id_of};
use time::Duration;

fn reply(action_id: &str, text: Option<&str>) -> ActionRequest {
    ActionRequest {
        action_id: action_id.to_owned(),
        value: text.map(str::to_owned),
        via: AnsweredVia::Mac,
    }
}

#[tokio::test]
async fn an_action_id_the_message_does_not_have_is_refused() {
    // Carried items 1 and 4: unknown ids, and ids of another message's
    // buttons, never answer.
    let hub = TestHub::start();
    let prompt = id_of(&hub.create(blocking_prompt()).await);
    let question = id_of(
        &hub.create(json!({
            "type": "decision_required", "title": "Which channel?", "awaitsResponse": true,
            "conversation": { "id": "conversation-2" },
            "actions": [{ "id": "pick-apns", "label": "APNs", "kind": "choice", "value": "apns" }]
        }))
        .await,
    );
    assert!(matches!(
        hub.press(&prompt, "nope").await,
        Err(HubError::UnknownAction)
    ));
    assert!(matches!(
        hub.press(&prompt, "pick-apns").await,
        Err(HubError::UnknownAction)
    ));
    // Done is not a button of a decision.
    assert!(matches!(
        hub.press(&prompt, "done").await,
        Err(HubError::UnknownAction)
    ));
    assert_eq!(hub.status_of(&prompt).await, ResponseStatus::Pending);
    assert_eq!(hub.status_of(&question).await, ResponseStatus::Pending);
    hub.stop().await;
}

#[tokio::test]
async fn the_hub_checks_what_is_offered_before_pressing() {
    let hub = TestHub::start();
    // No waiter: approve is a button but not offered.
    let id = id_of(
        &hub.create(json!({
            "type": "decision_required", "blocking": "blocking", "title": "Allow Bash to run the tests?",
            "actions": [{ "id": "allow", "label": "Allow", "kind": "approve" }]
        }))
        .await,
    );
    assert!(matches!(
        hub.press(&id, "allow").await,
        Err(HubError::ActionNotOffered)
    ));
    assert_eq!(hub.status_of(&id).await, ResponseStatus::Pending);
    hub.stop().await;
}

#[tokio::test]
async fn a_reply_needs_text_and_returns_it() {
    let hub = TestHub::start();
    let id = id_of(&hub.create(blocking_prompt()).await);
    assert!(matches!(
        hub.handle
            .take_action(id.clone(), reply("other", None))
            .await,
        Err(HubError::InvalidAnswerValue(_))
    ));
    assert!(matches!(
        hub.handle
            .take_action(id.clone(), reply("other", Some("  ")))
            .await,
        Err(HubError::InvalidAnswerValue(_))
    ));
    hub.handle
        .take_action(id.clone(), reply("other", Some("Run only the hub tests")))
        .await
        .unwrap();
    let response = hub.poll(&id, StdDuration::ZERO).await;
    assert_eq!(response.action_kind, Some(ActionKind::Reply));
    assert_eq!(response.value.as_deref(), Some("Run only the hub tests"));
    hub.stop().await;
}

#[tokio::test]
async fn a_deferred_item_never_repings_or_pops_up_again() {
    // Carried item 3.
    let hub = TestHub::start();
    let id = id_of(&hub.create(deferrable_question()).await);
    hub.press(&id, "defer").await.unwrap();
    let message = hub.handle.message(id.clone()).await.unwrap();
    assert_eq!(message.blocking, Blocking::NonBlocking);
    let events = hub.recorder.events();
    assert!(events.iter().any(|event| matches!(
        event,
        HubEvent::Updated { message, take_down, .. } if message.id == id && take_down.popup
    )));
    hub.recorder.clear();

    // Past every re-ping the deferrable cell had (10 min, twice), while the
    // sender keeps polling.
    for _ in 0..6 {
        hub.poll(&id, StdDuration::ZERO).await;
        hub.advance(Duration::minutes(5)).await;
    }
    // Other messages in the conversation come and go.
    hub.create(json!({ "type": "decision_required", "title": "Rename the crate later?" }))
        .await;
    let events = hub.recorder.events();
    let about_it = |event: &HubEvent| match event {
        HubEvent::Repinged { message_id, .. } => message_id == &id,
        HubEvent::Delivered { message, .. } => message.id == id,
        _ => false,
    };
    assert_eq!(count(&events, about_it), 0, "{events:#?}");
    // Defer is gone from the buttons; the real answer still closes it.
    let offered = hub.handle.offered_actions(id.clone()).await.unwrap();
    assert!(
        offered
            .iter()
            .all(|action| action.kind != ActionKind::Defer)
    );
    hub.poll(&id, StdDuration::ZERO).await;
    hub.press(&id, "apns").await.unwrap();
    assert_eq!(hub.status_of(&id).await, ResponseStatus::Answered);
    hub.stop().await;
}

#[tokio::test]
async fn defer_twice_is_refused() {
    let hub = TestHub::start();
    let id = id_of(&hub.create(deferrable_question()).await);
    hub.press(&id, "defer").await.unwrap();
    assert!(matches!(
        hub.press(&id, "defer").await,
        Err(HubError::ActionNotOffered)
    ));
    hub.stop().await;
}

#[tokio::test]
async fn another_source_cannot_rescind_or_move_a_message() {
    // Carried item 2.
    let hub = TestHub::start();
    let id = id_of(&hub.create(blocking_prompt()).await);
    let refused = hub
        .handle
        .rescind(
            id.clone(),
            RescindReason::Obsolete,
            Requester::agent("codex"),
        )
        .await;
    assert!(matches!(refused, Err(HubError::NotOwner)));
    let other_conversation = Requester {
        agent: "claude-code".to_owned(),
        conversation_id: Some("conversation-9".to_owned()),
    };
    let refused = hub
        .handle
        .rescind(id.clone(), RescindReason::Obsolete, other_conversation)
        .await;
    assert!(matches!(refused, Err(HubError::NotOwner)));
    let refused = hub
        .handle
        .move_message(
            id.clone(),
            MessageThreadMove {
                thread: None,
                topic: Some(serde_json::from_value(json!({ "key": "elsewhere" })).unwrap()),
            },
            Requester::agent("codex"),
        )
        .await;
    assert!(matches!(refused, Err(HubError::NotOwner)));
    assert_eq!(hub.status_of(&id).await, ResponseStatus::Pending);

    let own = Requester {
        agent: "claude-code".to_owned(),
        conversation_id: Some("conversation-1".to_owned()),
    };
    let response = hub
        .handle
        .rescind(id.clone(), RescindReason::ResolvedElsewhere, own.clone())
        .await
        .unwrap();
    assert_eq!(response.status, ResponseStatus::Rescinded);
    assert!(matches!(
        hub.handle
            .rescind(id, RescindReason::ResolvedElsewhere, own)
            .await,
        Err(HubError::MessageClosed { .. })
    ));
    hub.stop().await;
}

#[tokio::test]
async fn a_blocking_message_repings_every_five_minutes_at_most_three_times_while_open() {
    let hub = TestHub::start();
    let id = id_of(&hub.create(blocking_prompt()).await);
    let repings = |hub: &TestHub| {
        count(
            &hub.recorder.events(),
            |event| matches!(event, HubEvent::Repinged { message_id, .. } if message_id == &id),
        )
    };
    for minute in 1..=30 {
        hub.poll(&id, StdDuration::ZERO).await;
        hub.advance(Duration::minutes(1)).await;
        if minute == 4 {
            assert_eq!(repings(&hub), 0);
        }
    }
    assert_eq!(repings(&hub), 3);
    hub.stop().await;
}

#[tokio::test]
async fn acknowledging_stops_repings_and_a_snooze_returns_with_one_reping() {
    let hub = TestHub::start();
    let id = id_of(&hub.create(blocking_prompt()).await);
    hub.handle.snooze(id.clone(), Some(20)).await.unwrap();
    hub.recorder.clear();
    hub.advance(Duration::minutes(19)).await;
    let repinged =
        |events: &[HubEvent]| count(events, |event| matches!(event, HubEvent::Repinged { .. }));
    assert_eq!(repinged(&hub.recorder.events()), 0);
    hub.advance(Duration::minutes(1)).await;
    assert_eq!(repinged(&hub.recorder.events()), 1);
    let message = hub.handle.message(id).await.unwrap();
    assert_eq!(
        message.lifecycle.state,
        gentbox_core::lifecycle::LifecycleState::Open
    );
    hub.stop().await;
}

#[tokio::test]
async fn a_message_expires_at_its_time_and_its_popup_closes() {
    let hub = TestHub::start();
    let id = id_of(&hub.create(blocking_prompt()).await);
    // Blocking messages live 24 h by default.
    hub.advance(Duration::hours(23)).await;
    assert_eq!(hub.status_of(&id).await, ResponseStatus::Pending);
    hub.advance(Duration::hours(1)).await;
    assert_eq!(hub.status_of(&id).await, ResponseStatus::Expired);
    assert!(hub.recorder.events().iter().any(|event| matches!(
        event,
        HubEvent::Closed { message_id, take_down, .. } if message_id == &id && take_down.popup
    )));
    hub.stop().await;
}

#[tokio::test]
async fn ticks_are_stored_and_never_reach_the_sender() {
    let hub = TestHub::start();
    let id = id_of(
        &hub.create(json!({
            "type": "action_required", "blocking": "blocking", "title": "Sign in to npm",
            "awaitsResponse": true,
            "steps": [{ "text": "Open a terminal" }, { "text": "Sign in to npm" }]
        }))
        .await,
    );
    let poll = match hub.handle.begin_wait(id.clone()).await.unwrap() {
        gentbox_hub::WaitStart::Waiting(pending) => {
            tokio::spawn(pending.wait(StdDuration::from_millis(300)))
        }
        gentbox_hub::WaitStart::Ready(response) => panic!("{response:?}"),
    };
    let ticked = hub.handle.tick_step(id.clone(), 1, true).await.unwrap();
    assert_eq!(ticked.steps_completed, [1]);
    assert_eq!(poll.await.unwrap().status, ResponseStatus::Pending);
    assert!(matches!(
        hub.handle.tick_step(id.clone(), 5, true).await,
        Err(HubError::UnknownStep(5))
    ));
    assert_eq!(hub.handle.message(id).await.unwrap().steps_completed, [1]);
    hub.stop().await;
}

#[tokio::test]
async fn an_action_required_message_is_closed_with_done_never_cleared() {
    let hub = TestHub::start();
    let id = id_of(&hub.create(support::action_without_waiter()).await);
    assert!(matches!(
        hub.handle.dismiss(id.clone()).await,
        Err(HubError::UseDoneInstead)
    ));
    hub.stop().await;
}

#[tokio::test]
async fn another_agent_cannot_move_a_thread_holding_messages_it_does_not_own() {
    let hub = TestHub::start();
    let posted = hub.create(blocking_prompt()).await;
    let request = || -> gentbox_hub::ThreadMove {
        serde_json::from_value(json!({ "topic": { "key": "elsewhere" } })).unwrap()
    };
    let refused = hub
        .handle
        .move_thread(
            posted.thread_id.clone(),
            request(),
            Requester::agent("codex"),
        )
        .await;
    assert!(matches!(refused, Err(HubError::NotOwner)), "{refused:?}");
    let message = hub.handle.message(posted.id.clone()).await.unwrap();
    assert_eq!(message.topic_id, posted.topic_id);
    hub.handle
        .move_thread(posted.thread_id, request(), Requester::agent("claude-code"))
        .await
        .unwrap();
    hub.stop().await;
}

#[tokio::test]
async fn a_poll_answered_deferred_at_once_still_lapses_and_the_ui_hears_it() {
    let hub = TestHub::start();
    let id = id_of(&hub.create(deferrable_question()).await);
    hub.poll(&id, StdDuration::ZERO).await;
    hub.advance(Duration::seconds(5)).await;
    hub.press(&id, "defer").await.unwrap();
    hub.advance(Duration::milliseconds(4500)).await;
    assert_eq!(hub.status_of(&id).await, ResponseStatus::Deferred);
    // The lapse timer of the first poll fires while the second still counts.
    hub.advance(Duration::seconds(1)).await;
    assert!(hub.handle.has_live_waiter(id.clone()).await.unwrap());
    hub.recorder.clear();
    hub.advance(Duration::seconds(10)).await;
    assert!(!hub.handle.has_live_waiter(id.clone()).await.unwrap());
    let lapses = count(
        &hub.recorder.events(),
        |event| matches!(event, HubEvent::WaiterChanged { message_id, has_live_waiter: false, .. } if message_id == &id),
    );
    assert_eq!(lapses, 1);
    hub.stop().await;
}

#[tokio::test]
async fn every_poll_in_flight_when_you_defer_hears_deferred() {
    let hub = TestHub::start();
    let id = id_of(&hub.create(deferrable_question()).await);
    let start = |hub: &TestHub| {
        let handle = hub.handle.clone();
        let id = id.clone();
        async move {
            match handle.begin_wait(id).await.unwrap() {
                gentbox_hub::WaitStart::Waiting(pending) => pending,
                gentbox_hub::WaitStart::Ready(response) => panic!("{response:?}"),
            }
        }
    };
    let first = start(&hub).await;
    let second = start(&hub).await;
    hub.press(&id, "defer").await.unwrap();
    // The first poll reports and ends before the second reads its value.
    let first = first.wait(StdDuration::from_secs(5)).await;
    assert_eq!(first.status, ResponseStatus::Deferred);
    hub.handle.run_due_timers().await.unwrap();
    let second = second.wait(StdDuration::from_secs(5)).await;
    assert_eq!(second.status, ResponseStatus::Deferred);
    // Once reported, the next poll waits for the real answer.
    assert_eq!(hub.status_of(&id).await, ResponseStatus::Pending);
    hub.stop().await;
}

#[tokio::test]
async fn a_poll_dropped_before_it_delivers_deferred_leaves_it_for_the_next_poll() {
    let hub = TestHub::start();
    let id = id_of(&hub.create(deferrable_question()).await);
    let pending = match hub.handle.begin_wait(id.clone()).await.unwrap() {
        gentbox_hub::WaitStart::Waiting(pending) => pending,
        gentbox_hub::WaitStart::Ready(response) => panic!("{response:?}"),
    };
    hub.press(&id, "defer").await.unwrap();
    // The client hung up: the poll ends without handing anything over.
    drop(pending);
    hub.handle.run_due_timers().await.unwrap();
    assert_eq!(hub.status_of(&id).await, ResponseStatus::Deferred);
    assert_eq!(hub.status_of(&id).await, ResponseStatus::Pending);
    hub.stop().await;
}

#[tokio::test]
async fn waiter_state_of_a_message_closed_inside_its_live_window_is_forgotten() {
    let hub = TestHub::start();
    let id = id_of(&hub.create(blocking_prompt()).await);
    hub.press(&id, "allow").await.unwrap();
    hub.advance(Duration::seconds(10)).await;
    assert_eq!(hub.handle.tracked_waiters().await.unwrap(), 0);
    hub.stop().await;
}
