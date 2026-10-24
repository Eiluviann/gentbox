//! Long-poll (architecture §5.2, §9.1): pending, answered mid-wait, first
//! answer wins, every terminal state wakes the waiter, Defer once, the live
//! waiter's lapse, and what a swept id answers.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::time::Duration as StdDuration;

use gentbox_core::ids::{ConversationKey, MessageId};
use gentbox_core::message::ActionKind;
use gentbox_core::rescind::RescindReason;
use gentbox_core::response::{AnsweredVia, ResponseStatus};
use gentbox_hub::{ActionRequest, HubError, HubEvent, Requester, WaitStart};
use serde_json::json;
use support::{TestHub, action_without_waiter, blocking_prompt, count, deferrable_question, id_of};
use time::Duration;

const LONG: StdDuration = StdDuration::from_secs(5);

async fn waiting(
    hub: &TestHub,
    id: &MessageId,
) -> tokio::task::JoinHandle<gentbox_core::response::ActionResponse> {
    match hub.handle.begin_wait(id.clone()).await.unwrap() {
        WaitStart::Waiting(pending) => tokio::spawn(pending.wait(LONG)),
        WaitStart::Ready(response) => panic!("expected a pending wait, got {response:?}"),
    }
}

#[tokio::test]
async fn a_wait_with_no_answer_ends_pending() {
    let hub = TestHub::start();
    let id = id_of(&hub.create(blocking_prompt()).await);
    let response = hub.poll(&id, StdDuration::from_millis(50)).await;
    assert_eq!(response.status, ResponseStatus::Pending);
    assert_eq!(response.message_id, id);
    hub.stop().await;
}

#[tokio::test]
async fn an_answer_injected_through_the_handle_wakes_a_waiting_poll() {
    let hub = TestHub::start();
    let id = id_of(&hub.create(blocking_prompt()).await);
    let poll = waiting(&hub, &id).await;
    hub.press(&id, "allow").await.unwrap();
    let response = poll.await.unwrap();
    assert_eq!(response.status, ResponseStatus::Answered);
    assert_eq!(response.action_id.as_deref(), Some("allow"));
    assert_eq!(response.action_kind, Some(ActionKind::Approve));
    assert_eq!(response.answered_via, Some(AnsweredVia::Mac));
    // A later poll gets the same answer at once.
    assert_eq!(hub.status_of(&id).await, ResponseStatus::Answered);
    hub.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_concurrent_answers_one_wins_and_the_other_is_refused() {
    for _ in 0..20 {
        let hub = TestHub::start();
        let id = id_of(&hub.create(blocking_prompt()).await);
        let poll = waiting(&hub, &id).await;
        let (first, second) = tokio::join!(hub.press(&id, "allow"), hub.press(&id, "deny"));
        let results = [first, second];
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        let refused = results
            .iter()
            .find_map(|result| result.as_ref().err())
            .unwrap();
        assert!(
            matches!(refused, HubError::MessageClosed { .. }),
            "{refused:?}"
        );
        let response = poll.await.unwrap();
        assert_eq!(response.status, ResponseStatus::Answered);
        let winner = if results[0].is_ok() { "allow" } else { "deny" };
        assert_eq!(response.action_id.as_deref(), Some(winner));
        hub.stop().await;
    }
}

#[tokio::test]
async fn rescind_dismiss_and_expiry_each_wake_the_waiter() {
    let hub = TestHub::start();

    let rescinded = id_of(&hub.create(blocking_prompt()).await);
    let poll = waiting(&hub, &rescinded).await;
    hub.handle
        .rescind(
            rescinded.clone(),
            RescindReason::ResolvedElsewhere,
            Requester::agent("claude-code"),
        )
        .await
        .unwrap();
    let response = poll.await.unwrap();
    assert_eq!(response.status, ResponseStatus::Rescinded);
    assert_eq!(
        response.rescind_reason,
        Some(RescindReason::ResolvedElsewhere)
    );

    let dismissed = id_of(
        &hub.create(json!({
            "type": "decision_required", "title": "Pick a release name", "awaitsResponse": true,
            "conversation": { "id": "conversation-2" }
        }))
        .await,
    );
    let poll = waiting(&hub, &dismissed).await;
    hub.handle.dismiss(dismissed.clone()).await.unwrap();
    assert_eq!(poll.await.unwrap().status, ResponseStatus::Dismissed);

    let expiring = id_of(
        &hub.create(json!({
            "type": "decision_required", "blocking": "blocking", "title": "Approve the deploy?",
            "awaitsResponse": true, "conversation": { "id": "conversation-3" },
            "expiresAt": "2026-10-24T12:05:00Z"
        }))
        .await,
    );
    let poll = waiting(&hub, &expiring).await;
    hub.advance(Duration::minutes(5)).await;
    assert_eq!(poll.await.unwrap().status, ResponseStatus::Expired);
    hub.stop().await;
}

#[tokio::test]
async fn the_conversation_rescind_route_wakes_waiters_with_rescinded() {
    let hub = TestHub::start();
    let mut prompt = blocking_prompt();
    prompt["correlationKey"] = json!("permission:main:abc");
    let id = id_of(&hub.create(prompt).await);
    let poll = waiting(&hub, &id).await;
    let rescinded = hub
        .handle
        .rescind_by_correlation_keys(
            ConversationKey::new("claude-code", "conversation-1"),
            serde_json::from_value(
                json!({ "reason": "obsolete", "correlationKeys": ["permission:main:abc"] }),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rescinded, [id]);
    let response = poll.await.unwrap();
    assert_eq!(response.status, ResponseStatus::Rescinded);
    assert_eq!(response.rescind_reason, Some(RescindReason::Obsolete));
    hub.stop().await;
}

#[tokio::test]
async fn snooze_and_open_never_wake_the_waiter() {
    let hub = TestHub::start();
    let id = id_of(&hub.create(blocking_prompt()).await);
    let poll = waiting(&hub, &id).await;
    hub.handle.acknowledge(id.clone()).await.unwrap();
    hub.handle.snooze(id.clone(), Some(30)).await.unwrap();
    hub.handle.tick_step(id.clone(), 0, true).await.unwrap_err();
    tokio::time::sleep(StdDuration::from_millis(100)).await;
    assert!(!poll.is_finished(), "open and snooze reached the sender");
    hub.press(&id, "deny").await.unwrap();
    assert_eq!(poll.await.unwrap().status, ResponseStatus::Answered);
    hub.stop().await;
}

#[tokio::test]
async fn defer_wakes_the_waiter_with_deferred_once_then_pending_until_the_real_answer() {
    let hub = TestHub::start();
    let id = id_of(&hub.create(deferrable_question()).await);
    let poll = waiting(&hub, &id).await;
    hub.press(&id, "defer").await.unwrap();
    assert_eq!(poll.await.unwrap().status, ResponseStatus::Deferred);

    assert_eq!(hub.status_of(&id).await, ResponseStatus::Pending);
    assert_eq!(hub.status_of(&id).await, ResponseStatus::Pending);
    let message = hub.handle.message(id.clone()).await.unwrap();
    assert_eq!(
        message.blocking,
        gentbox_core::message::Blocking::NonBlocking
    );

    let poll = waiting(&hub, &id).await;
    hub.press(&id, "ntfy").await.unwrap();
    let answered = poll.await.unwrap();
    assert_eq!(answered.status, ResponseStatus::Answered);
    assert_eq!(answered.value.as_deref(), Some("ntfy"));
    hub.stop().await;
}

#[tokio::test]
async fn a_defer_chosen_while_nobody_polls_reaches_the_next_poll_once() {
    let hub = TestHub::start();
    let id = id_of(&hub.create(deferrable_question()).await);
    // The sender polls, then is between polls when you defer.
    hub.poll(&id, StdDuration::ZERO).await;
    hub.press(&id, "defer").await.unwrap();
    assert_eq!(hub.status_of(&id).await, ResponseStatus::Deferred);
    assert_eq!(hub.status_of(&id).await, ResponseStatus::Pending);
    hub.stop().await;
}

#[tokio::test]
async fn the_live_waiter_lapses_ten_seconds_after_the_last_poll() {
    let hub = TestHub::start();
    let id = id_of(&hub.create(blocking_prompt()).await);
    hub.poll(&id, StdDuration::ZERO).await;
    hub.advance(Duration::seconds(9)).await;
    assert!(hub.handle.has_live_waiter(id.clone()).await.unwrap());
    hub.recorder.clear();

    hub.advance(Duration::seconds(1)).await;
    assert!(!hub.handle.has_live_waiter(id.clone()).await.unwrap());
    let events = hub.recorder.events();
    let lapsed = events
        .iter()
        .find_map(|event| match event {
            HubEvent::WaiterChanged {
                message_id,
                has_live_waiter,
                offered_actions,
            } if message_id == &id => Some((*has_live_waiter, offered_actions.clone())),
            _ => None,
        })
        .expect("the UI hears the lapse");
    assert!(!lapsed.0);
    assert!(
        lapsed.1.iter().all(|action| !action.kind.is_answer()),
        "answer buttons go with the waiter: {:?}",
        lapsed.1
    );
    // Without a live waiter an answer is no longer offered.
    assert!(matches!(
        hub.press(&id, "allow").await,
        Err(HubError::ActionNotOffered)
    ));
    hub.stop().await;
}

#[tokio::test]
async fn a_poll_in_flight_keeps_the_waiter_live_however_long_it_holds() {
    let hub = TestHub::start();
    let id = id_of(&hub.create(blocking_prompt()).await);
    let poll = waiting(&hub, &id).await;
    hub.advance(Duration::seconds(50)).await;
    assert!(hub.handle.has_live_waiter(id.clone()).await.unwrap());
    hub.press(&id, "allow").await.unwrap();
    poll.await.unwrap();
    hub.stop().await;
}

#[tokio::test]
async fn a_new_message_whose_sender_waits_counts_as_live_until_its_first_poll_is_late() {
    let hub = TestHub::start();
    let id = id_of(&hub.create(blocking_prompt()).await);
    assert!(hub.handle.has_live_waiter(id.clone()).await.unwrap());
    hub.advance(Duration::seconds(10)).await;
    assert!(!hub.handle.has_live_waiter(id.clone()).await.unwrap());
    hub.stop().await;
}

#[tokio::test]
async fn done_without_a_waiter_closes_the_item_and_a_later_poll_gets_done() {
    let hub = TestHub::start();
    let response = hub.create(action_without_waiter()).await;
    assert!(!response.should_wait);
    let id = id_of(&response);
    assert!(!hub.handle.has_live_waiter(id.clone()).await.unwrap());
    let offered = hub.handle.offered_actions(id.clone()).await.unwrap();
    assert!(offered.iter().any(|action| action.kind == ActionKind::Done));

    hub.press(&id, "done").await.unwrap();
    let message = hub.handle.message(id.clone()).await.unwrap();
    assert_eq!(
        message.lifecycle.state,
        gentbox_core::lifecycle::LifecycleState::Answered
    );
    let later = hub.poll(&id, StdDuration::ZERO).await;
    assert_eq!(later.status, ResponseStatus::Answered);
    assert_eq!(later.action_kind, Some(ActionKind::Done));
    assert_eq!(later.action_id.as_deref(), Some("done"));
    hub.stop().await;
}

#[tokio::test]
async fn a_swept_message_answers_gone_and_an_unknown_id_is_not_found() {
    let hub = TestHub::start();
    let id = id_of(&hub.create(action_without_waiter()).await);
    hub.press(&id, "done").await.unwrap();
    // The daily sweep runs as the clock passes it.
    hub.advance(Duration::days(31)).await;
    assert!(matches!(
        hub.handle.message(id.clone()).await,
        Err(HubError::MessageNotFound(_))
    ));

    let response = hub.poll(&id, LONG).await;
    assert_eq!(response.status, ResponseStatus::Gone);
    assert_eq!(response.message_id, id);
    assert!(response.action_kind.is_none());

    let unknown = MessageId::new("01JBM3Z8Q7Y4K2N6P0R5T9VWXA");
    assert!(matches!(
        hub.handle.wait_for_response(unknown, LONG).await,
        Err(HubError::MessageNotFound(_))
    ));
    hub.stop().await;
}

#[tokio::test]
async fn a_poll_whose_client_goes_away_still_ends_the_wait() {
    let hub = TestHub::start();
    let id = id_of(&hub.create(blocking_prompt()).await);
    let poll = waiting(&hub, &id).await;
    poll.abort();
    let _ = poll.await;
    // The guard told the hub: the waiter lapses 10 s later.
    hub.advance(Duration::seconds(10)).await;
    assert!(!hub.handle.has_live_waiter(id.clone()).await.unwrap());
    hub.stop().await;
}

#[tokio::test]
async fn the_ui_hears_answers_close_the_popup() {
    let hub = TestHub::start();
    let id = id_of(&hub.create(blocking_prompt()).await);
    hub.handle
        .take_action(
            id.clone(),
            ActionRequest {
                action_id: "other".to_owned(),
                value: Some("Run only the hub tests".to_owned()),
                via: AnsweredVia::Mac,
            },
        )
        .await
        .unwrap();
    let events = hub.recorder.events();
    assert_eq!(
        count(&events, |event| matches!(
            event,
            HubEvent::Closed { message_id, take_down, .. } if message_id == &id && take_down.popup
        )),
        1
    );
    let response = hub.poll(&id, StdDuration::ZERO).await;
    assert_eq!(response.value.as_deref(), Some("Run only the hub tests"));
    hub.stop().await;
}
