//! The ingest pipeline through the hub (architecture §5.1, §7): delivery,
//! idempotency, supersede, fold, dedupe, completion and progress, and
//! `shouldWait` without a relay.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::time::Duration as StdDuration;

use gentbox_core::ids::ConversationKey;
use gentbox_core::lifecycle::LifecycleState;
use gentbox_core::message::Blocking;
use gentbox_core::policy::{MacSurface, PhonePlan};
use gentbox_core::rescind::{ChannelEffect, RescindReason};
use gentbox_core::response::ResponseStatus;
use gentbox_hub::{HubEvent, IngestStatus, Requester};
use gentbox_store::MessageFilter;
use serde_json::json;
use support::{TestHub, action_without_waiter, blocking_prompt, count, id_of};
use time::Duration;

#[tokio::test]
async fn without_a_relay_a_message_goes_to_the_mac_only_and_should_wait_follows_the_mac() {
    let hub = TestHub::start();
    let response = hub.create(blocking_prompt()).await;
    assert!(response.should_wait);
    assert_eq!(response.state, LifecycleState::Open);
    let events = hub.recorder.events();
    let plan = events
        .iter()
        .find_map(|event| match event {
            HubEvent::Delivered { plan, .. } => Some(plan.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(plan.phone, PhonePlan::None);
    assert_eq!(plan.mac, MacSurface::PersistentPopup);

    // A sender that does not intend to poll is not told to.
    let notification = hub
        .create(json!({
            "type": "decision_required", "blocking": "blocking", "title": "Allow Bash to list files?",
            "conversation": { "id": "conversation-2" }
        }))
        .await;
    assert!(!notification.should_wait);
    hub.stop().await;
}

#[tokio::test]
async fn a_hub_without_an_answer_channel_tells_senders_not_to_wait() {
    let hub = TestHub::start_with(|config| config.has_answer_channel = false);
    let response = hub.create(blocking_prompt()).await;
    assert!(!response.should_wait);
    hub.stop().await;
}

#[tokio::test]
async fn an_idempotent_retry_returns_the_stored_message_and_changes_nothing() {
    let hub = TestHub::start();
    let mut prompt = blocking_prompt();
    prompt["idempotencyKey"] = json!("retry-1");
    let first = hub.create(prompt.clone()).await;
    let second = hub.post(prompt).await;
    assert_eq!(second.status, IngestStatus::Existing);
    assert_eq!(second.response.id, first.id);
    let all = hub
        .handle
        .list_messages(MessageFilter::default())
        .await
        .unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].repeat_count, 0);
    hub.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_posts_with_one_idempotency_key_store_one_message() {
    let hub = TestHub::start();
    let mut prompt = blocking_prompt();
    prompt["idempotencyKey"] = json!("race");
    let posts = (0..16).map(|_| {
        let handle = hub.handle.clone();
        let input = support::message(prompt.clone());
        async move { handle.ingest(input).await }
    });
    let outcomes = futures_join_all(posts).await;
    let created = outcomes
        .iter()
        .filter(|outcome| outcome.as_ref().unwrap().status == IngestStatus::Created)
        .count();
    assert_eq!(created, 1);
    let ids: std::collections::BTreeSet<_> = outcomes
        .iter()
        .map(|outcome| outcome.as_ref().unwrap().response.id.clone())
        .collect();
    assert_eq!(ids.len(), 1);
    hub.stop().await;
}

#[tokio::test]
async fn a_supersede_takes_the_slot_in_place_and_the_old_waiter_hears_superseded() {
    let hub = TestHub::start();
    let mut first = blocking_prompt();
    first["correlationKey"] = json!("permission:main:abc");
    let old = id_of(&hub.create(first.clone()).await);
    hub.recorder.clear();
    first["title"] = json!("Allow Bash to run every test?");
    let new = hub.create(first).await;
    assert_ne!(new.id, old);

    assert_eq!(
        hub.poll(&old, StdDuration::ZERO).await.rescind_reason,
        Some(RescindReason::Superseded)
    );
    let events = hub.recorder.events();
    assert!(matches!(
        &events[0],
        HubEvent::Delivered { replaces: Some(replaced), pings: false, .. } if replaced == &old
    ));
    assert!(events.iter().any(|event| matches!(
        event,
        HubEvent::Withdrawn(rescission)
            if rescission.message_id == old
                && rescission.effects.contains(&ChannelEffect::FoldIntoSuccessor(new.id.clone()))
    )));
    hub.stop().await;
}

#[tokio::test]
async fn a_muted_message_that_will_not_be_waited_on_folds_into_a_live_waiter() {
    // Carried item 7: the hub passes `awaitsResponse && shouldWait`. A muted,
    // non-urgent message with awaitsResponse gets shouldWait false, so it must
    // not displace the item whose sender is waiting.
    let hub = TestHub::start();
    let mut prompt = blocking_prompt();
    prompt["correlationKey"] = json!("permission:main:abc");
    let live = hub.create(prompt.clone()).await;
    hub.handle
        .mute_thread(live.thread_id.clone(), true)
        .await
        .unwrap();

    prompt["title"] = json!("Allow Bash to run every test?");
    let muted = hub.post(prompt).await;
    assert_eq!(muted.status, IngestStatus::Existing);
    assert_eq!(muted.response.id, live.id);
    assert!(!muted.response.should_wait);
    assert_eq!(hub.status_of(&live.id).await, ResponseStatus::Pending);
    let stored = hub.handle.message(live.id.clone()).await.unwrap();
    assert_eq!(stored.repeat_count, 1);
    hub.stop().await;
}

#[tokio::test]
async fn a_blocking_notification_without_a_waiter_folds_into_the_waiting_prompt() {
    let hub = TestHub::start();
    let live = hub.create(blocking_prompt()).await;
    let folded = hub
        .post(json!({ "event": "claude-code.notification.permission_prompt", "title": "Claude Code needs your permission" }))
        .await;
    assert_eq!(folded.status, IngestStatus::Existing);
    assert_eq!(folded.response.id, live.id);
    assert!(!folded.response.should_wait);
    let events = hub.recorder.events();
    assert_eq!(
        count(&events, |event| matches!(event, HubEvent::Delivered { .. })),
        1
    );
    assert_eq!(
        count(&events, |event| matches!(event, HubEvent::Repeated { .. })),
        1
    );
    hub.stop().await;
}

#[tokio::test]
async fn an_identical_message_within_ten_seconds_is_a_repeat() {
    let hub = TestHub::start();
    let fields = json!({
        "type": "task_completed", "title": "Ingest tests pass", "correlationKey": "stop"
    });
    let first = hub.create(fields.clone()).await;
    hub.advance(Duration::seconds(5)).await;
    let repeat = hub.post(fields.clone()).await;
    assert_eq!(repeat.status, IngestStatus::Existing);
    assert_eq!(repeat.response.id, first.id);
    hub.advance(Duration::seconds(10)).await;
    assert_eq!(hub.post(fields).await.status, IngestStatus::Created);
    hub.stop().await;
}

#[tokio::test]
async fn a_completion_rescinds_blocking_items_without_a_waiter_and_keeps_the_rest() {
    let hub = TestHub::start();
    // Posted before the waiting prompt: afterwards it would fold into it.
    let stale = hub
        .create(json!({ "type": "decision_required", "blocking": "deferrable", "title": "Pick the next phase" }))
        .await;
    let waiting = hub.create(blocking_prompt()).await;
    let note = hub
        .create(json!({ "type": "decision_required", "title": "Rename the crate later?" }))
        .await;
    hub.create(json!({ "type": "task_completed", "title": "Phase 3 is done" }))
        .await;
    assert_eq!(hub.status_of(&stale.id).await, ResponseStatus::Rescinded);
    assert_eq!(
        hub.poll(&stale.id, StdDuration::ZERO).await.rescind_reason,
        Some(RescindReason::HandledInSession)
    );
    assert_eq!(hub.status_of(&waiting.id).await, ResponseStatus::Pending);
    assert_eq!(hub.status_of(&note.id).await, ResponseStatus::Pending);
    hub.stop().await;
}

#[tokio::test]
async fn progress_rescinds_the_idle_item_and_blocking_items_without_a_waiter() {
    let hub = TestHub::start();
    let idle = hub
        .create(json!({ "event": "claude-code.notification.idle_prompt", "title": "Claude Code is waiting for you", "correlationKey": "idle" }))
        .await;
    let stale = hub.create(action_without_waiter()).await;
    let waiting = hub.create(blocking_prompt()).await;
    let rescinded = hub
        .handle
        .progress(ConversationKey::new("claude-code", "conversation-1"))
        .await
        .unwrap();
    assert_eq!(rescinded.len(), 2);
    assert!(rescinded.contains(&idle.id) && rescinded.contains(&stale.id));
    for id in [&idle.id, &stale.id] {
        assert_eq!(
            hub.poll(id, StdDuration::ZERO).await.rescind_reason,
            Some(RescindReason::HandledInSession)
        );
    }
    assert_eq!(hub.status_of(&waiting.id).await, ResponseStatus::Pending);
    hub.stop().await;
}

#[tokio::test]
async fn an_explicit_rescind_keeps_the_reason_the_agent_gave() {
    let hub = TestHub::start();
    for reason in RescindReason::EXPLICIT {
        let posted = hub
            .create(json!({ "type": "decision_required", "title": format!("Ship {reason:?}?"), "correlationKey": format!("k-{reason:?}") }))
            .await;
        hub.handle
            .rescind(posted.id.clone(), reason, Requester::agent("claude-code"))
            .await
            .unwrap();
        assert_eq!(
            hub.poll(&posted.id, StdDuration::ZERO).await.rescind_reason,
            Some(reason)
        );
    }
    hub.stop().await;
}

#[tokio::test]
async fn coalescing_never_crosses_conversations_even_in_one_thread() {
    let hub = TestHub::start();
    let first = hub
        .create(json!({ "type": "decision_required", "blocking": "blocking", "title": "Allow the build?", "correlationKey": "k", "thread": { "key": "phase-3" }, "topic": { "key": "gentbox" } }))
        .await;
    let second = hub
        .create(json!({ "type": "decision_required", "blocking": "blocking", "title": "Allow the tests?", "correlationKey": "k", "thread": { "key": "phase-3" }, "topic": { "key": "gentbox" }, "conversation": { "id": "conversation-2" } }))
        .await;
    assert_eq!(first.thread_id, second.thread_id);
    assert_eq!(hub.status_of(&first.id).await, ResponseStatus::Pending);
    hub.stop().await;
}

#[tokio::test]
async fn the_pipeline_rewrites_a_command_title_and_warns() {
    let hub = TestHub::start();
    let response = hub
        .create(json!({ "type": "task_completed", "title": "cargo test -p ingest --nocapture", "summary": "Ingest tests pass. All green." }))
        .await;
    assert_eq!(
        response.warnings[0].code,
        gentbox_core::response::WarningCode::TitleLooksLikeCommand
    );
    let message = hub.handle.message(response.id).await.unwrap();
    assert_eq!(message.title, "Ingest tests pass");
    assert!(
        message
            .body
            .unwrap()
            .contains("cargo test -p ingest --nocapture")
    );
    hub.stop().await;
}

#[tokio::test]
async fn strict_modes_reject_per_source() {
    use gentbox_core::action_plan::ActionPlanMode;
    use gentbox_core::prepare::IngestSettings;
    use gentbox_core::title_guard::GuardMode;
    let hub = TestHub::start_with(|config| {
        config.sources.insert(
            "strict-adapter".to_owned(),
            IngestSettings {
                title_guard: GuardMode::Reject,
                action_plan: ActionPlanMode::Reject,
            },
        );
    });
    let strict = |fields: serde_json::Value| {
        let mut fields = fields;
        fields["source"] = json!({ "agent": "strict-adapter" });
        fields
    };
    let title = hub
        .handle
        .ingest(support::message(strict(
            json!({ "title": "git push --force" }),
        )))
        .await;
    assert!(matches!(title, Err(gentbox_hub::HubError::Rejected(_))));
    let plan = hub
        .handle
        .ingest(support::message(strict(
            json!({ "type": "action_required", "title": "Sign in to npm" }),
        )))
        .await;
    assert!(matches!(plan, Err(gentbox_hub::HubError::Rejected(_))));
    // Other sources keep the rewrite default.
    let lenient = hub
        .create(json!({ "type": "action_required", "title": "Sign in to npm" }))
        .await;
    assert_eq!(
        lenient.warnings[0].code,
        gentbox_core::response::WarningCode::StepsMissing
    );
    let stored = hub.handle.message(lenient.id).await.unwrap();
    assert_eq!(stored.steps.len(), 1);
    assert_eq!(stored.blocking, Blocking::NonBlocking);
    hub.stop().await;
}

async fn futures_join_all<F, T>(futures: impl Iterator<Item = F>) -> Vec<T>
where
    F: std::future::Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let handles: Vec<_> = futures.map(tokio::spawn).collect();
    let mut results = Vec::new();
    for handle in handles {
        results.push(handle.await.unwrap());
    }
    results
}

#[tokio::test]
async fn a_dedupe_that_brings_a_waiter_shows_the_answer_buttons() {
    let hub = TestHub::start();
    let mut prompt = blocking_prompt();
    prompt["correlationKey"] = json!("permission:main:abc");
    prompt["awaitsResponse"] = json!(false);
    let first = hub.create(prompt.clone()).await;
    assert!(!first.should_wait);
    hub.recorder.clear();

    prompt["awaitsResponse"] = json!(true);
    let repeat = hub.post(prompt).await;
    assert_eq!(repeat.status, IngestStatus::Existing);
    assert_eq!(repeat.response.id, first.id);
    assert!(repeat.response.should_wait);
    let shown = hub
        .recorder
        .events()
        .into_iter()
        .find_map(|event| match event {
            HubEvent::WaiterChanged {
                message_id,
                has_live_waiter: true,
                offered_actions,
            } if message_id == first.id => Some(offered_actions),
            _ => None,
        });
    let shown = shown.expect("the UI hears that a sender waits");
    assert!(shown.iter().any(|action| action.id == "allow"), "{shown:?}");
    hub.stop().await;
}
