//! A parked message (muted when it was sent, or timed out on a sender that was
//! told to collect the answer later) is exempt from the implicit clean-ups of
//! completion and progress. It closes only by the user's answer or dismiss, the
//! agent's own rescind, or expiry. A waiter that simply lapses stays eligible.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::time::Duration as StdDuration;

use gentbox_core::ids::ConversationKey;
use gentbox_core::rescind::RescindReason;
use gentbox_core::response::ResponseStatus;
use gentbox_core::topic::TopicCreate;
use gentbox_hub::{HubError, Requester};
use serde_json::json;
use support::{TestHub, blocking_prompt};
use time::Duration;

const CONVERSATION: (&str, &str) = ("claude-code", "conversation-1");

fn owner() -> Requester {
    Requester {
        agent: "claude-code".to_owned(),
        conversation_id: Some("conversation-1".to_owned()),
    }
}

async fn muted_ask(
    hub: &TestHub,
    extra: serde_json::Value,
) -> gentbox_core::response::IngestResponse {
    let request: TopicCreate =
        serde_json::from_value(json!({ "key": "release-1.4", "title": "Release 1.4" })).unwrap();
    let topic = hub
        .handle
        .create_topic("claude-code".to_owned(), request)
        .await
        .unwrap()
        .into_inner();
    hub.handle.mute_topic(topic.id, true).await.unwrap();
    let mut prompt = blocking_prompt();
    prompt["topic"] = json!({ "key": "release-1.4" });
    support::merge(&mut prompt, extra);
    hub.create(prompt).await
}

async fn clean_up(hub: &TestHub) {
    hub.handle
        .progress(ConversationKey::new(CONVERSATION.0, CONVERSATION.1))
        .await
        .unwrap();
    hub.create(json!({ "type": "task_completed", "title": "Phase 3 is done" }))
        .await;
}

#[tokio::test]
async fn a_muted_ask_is_parked_and_the_clean_ups_leave_it_alone() {
    let hub = TestHub::start();
    let ask = muted_ask(&hub, json!({})).await;
    assert!(ask.parked);
    assert!(
        hub.handle
            .message(ask.id.clone())
            .await
            .unwrap()
            .parked_at
            .is_some()
    );
    clean_up(&hub).await;
    assert_eq!(hub.status_of(&ask.id).await, ResponseStatus::Pending);
    hub.stop().await;
}

#[tokio::test]
async fn a_muted_message_that_awaits_nothing_is_not_parked() {
    let hub = TestHub::start();
    let ask = muted_ask(
        &hub,
        json!({ "awaitsResponse": false, "blocking": "deferrable", "actions": null }),
    )
    .await;
    assert!(!ask.parked);
    clean_up(&hub).await;
    assert_eq!(hub.status_of(&ask.id).await, ResponseStatus::Rescinded);
    hub.stop().await;
}

#[tokio::test]
async fn an_ask_that_timed_out_is_parked_by_the_senders_park_call() {
    let hub = TestHub::start();
    let ask = hub.create(blocking_prompt()).await;
    assert!(!ask.parked);
    let response = hub.handle.park(ask.id.clone(), owner()).await.unwrap();
    assert_eq!(response.status, ResponseStatus::Pending);
    assert!(
        hub.handle
            .message(ask.id.clone())
            .await
            .unwrap()
            .parked_at
            .is_some()
    );
    // The waiter lapses, then the session moves on: a parked ask stays.
    hub.advance(Duration::seconds(30)).await;
    clean_up(&hub).await;
    assert_eq!(hub.status_of(&ask.id).await, ResponseStatus::Pending);
    hub.stop().await;
}

#[tokio::test]
async fn a_lapsed_waiter_that_was_never_parked_is_still_cleared_as_handled_in_the_session() {
    // A hook killed by a permission answered in the terminal: nobody parks it,
    // and its stale popup must still clear.
    let hub = TestHub::start();
    let ask = hub.create(blocking_prompt()).await;
    hub.poll(&ask.id, StdDuration::ZERO).await;
    hub.advance(Duration::seconds(30)).await;
    clean_up(&hub).await;
    let response = hub.poll(&ask.id, StdDuration::ZERO).await;
    assert_eq!(response.status, ResponseStatus::Rescinded);
    assert_eq!(
        response.rescind_reason,
        Some(RescindReason::HandledInSession)
    );
    hub.stop().await;
}

#[tokio::test]
async fn a_parked_ask_still_closes_by_answer_dismiss_rescind_and_expiry() {
    let hub = TestHub::start();
    let answered = hub.create(blocking_prompt()).await;
    let dismissed = hub
        .create(json!({ "type": "decision_required", "blocking": "deferrable", "title": "Pick a name", "conversation": { "id": "c-2" }, "awaitsResponse": true }))
        .await;
    let rescinded = hub
        .create(json!({ "type": "decision_required", "blocking": "deferrable", "title": "Pick a color", "conversation": { "id": "c-3" }, "awaitsResponse": true }))
        .await;
    let expiring = hub
        .create(json!({ "type": "decision_required", "blocking": "blocking", "title": "Approve the deploy?", "conversation": { "id": "c-4" }, "awaitsResponse": true, "expiresAt": "2026-10-24T12:05:00Z" }))
        .await;
    let requester = |conversation: &str| Requester {
        agent: "claude-code".to_owned(),
        conversation_id: Some(conversation.to_owned()),
    };
    hub.handle
        .park(answered.id.clone(), requester("conversation-1"))
        .await
        .unwrap();
    hub.handle
        .park(dismissed.id.clone(), requester("c-2"))
        .await
        .unwrap();
    hub.handle
        .park(rescinded.id.clone(), requester("c-3"))
        .await
        .unwrap();
    hub.handle
        .park(expiring.id.clone(), requester("c-4"))
        .await
        .unwrap();

    hub.press(&answered.id, "allow").await.unwrap();
    assert_eq!(hub.status_of(&answered.id).await, ResponseStatus::Answered);

    hub.handle.dismiss(dismissed.id.clone()).await.unwrap();
    assert_eq!(
        hub.status_of(&dismissed.id).await,
        ResponseStatus::Dismissed
    );

    hub.handle
        .rescind(
            rescinded.id.clone(),
            RescindReason::ResolvedElsewhere,
            requester("c-3"),
        )
        .await
        .unwrap();
    let response = hub.poll(&rescinded.id, StdDuration::ZERO).await;
    assert_eq!(
        response.rescind_reason,
        Some(RescindReason::ResolvedElsewhere)
    );

    hub.advance(Duration::minutes(5)).await;
    assert_eq!(hub.status_of(&expiring.id).await, ResponseStatus::Expired);
    hub.stop().await;
}

#[tokio::test]
async fn only_the_owner_can_park_and_only_while_the_message_is_open() {
    let hub = TestHub::start();
    let ask = hub.create(blocking_prompt()).await;
    let other_agent = Requester::agent("codex");
    assert!(matches!(
        hub.handle.park(ask.id.clone(), other_agent).await,
        Err(HubError::NotOwner)
    ));
    let other_conversation = Requester {
        agent: "claude-code".to_owned(),
        conversation_id: Some("conversation-2".to_owned()),
    };
    assert!(matches!(
        hub.handle.park(ask.id.clone(), other_conversation).await,
        Err(HubError::NotOwner)
    ));
    assert!(
        hub.handle
            .message(ask.id.clone())
            .await
            .unwrap()
            .parked_at
            .is_none()
    );

    hub.handle.dismiss(ask.id.clone()).await.unwrap();
    assert!(matches!(
        hub.handle.park(ask.id.clone(), owner()).await,
        Err(HubError::MessageClosed { .. })
    ));
    hub.stop().await;
}

#[tokio::test]
async fn parking_twice_keeps_the_first_time() {
    let hub = TestHub::start();
    let ask = hub.create(blocking_prompt()).await;
    hub.handle.park(ask.id.clone(), owner()).await.unwrap();
    let first = hub.handle.message(ask.id.clone()).await.unwrap().parked_at;
    hub.advance(Duration::seconds(5)).await;
    hub.handle.park(ask.id.clone(), owner()).await.unwrap();
    assert_eq!(
        hub.handle.message(ask.id.clone()).await.unwrap().parked_at,
        first
    );
    hub.stop().await;
}

#[tokio::test]
async fn a_newer_message_with_the_same_key_still_supersedes_a_parked_ask() {
    let hub = TestHub::start();
    let first = muted_ask(&hub, json!({ "correlationKey": "ask-1" })).await;
    assert!(first.parked);
    let mut newer = blocking_prompt();
    newer["correlationKey"] = json!("ask-1");
    newer["title"] = json!("Allow Bash to run all the tests?");
    hub.create(newer).await;
    let response = hub.poll(&first.id, StdDuration::ZERO).await;
    assert_eq!(response.rescind_reason, Some(RescindReason::Superseded));
    hub.stop().await;
}
