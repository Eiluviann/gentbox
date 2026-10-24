//! Topics and threads through the hub (ADRs 0008, 0009): concurrent upserts,
//! takeover, close with and without withdrawing, reopen by a message, moves.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use gentbox_core::response::ResponseStatus;
use gentbox_core::topic::{CloseRequest, GroupStatus, ThreadCreate, TopicCreate};
use gentbox_hub::{HubError, Requester, ThreadMove, ThreadUpdate, TopicUpdate};
use gentbox_store::Upserted;
use serde_json::json;
use support::{TestHub, blocking_prompt};

fn topic_create(key: &str) -> TopicCreate {
    serde_json::from_value(json!({ "key": key, "title": "Release 1.4" })).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_upserts_of_one_key_create_one_topic_and_one_thread() {
    let hub = TestHub::start();
    let topics: Vec<_> = (0..16)
        .map(|_| {
            let handle = hub.handle.clone();
            tokio::spawn(async move {
                handle
                    .create_topic("claude-code".to_owned(), topic_create("release-1.4"))
                    .await
            })
        })
        .collect();
    let mut created = 0;
    let mut ids = std::collections::BTreeSet::new();
    for topic in topics {
        let topic = topic.await.unwrap().unwrap();
        created += usize::from(topic.was_created());
        ids.insert(topic.into_inner().id);
    }
    assert_eq!((created, ids.len()), (1, 1));

    let threads: Vec<_> = (0..16)
        .map(|_| {
            let handle = hub.handle.clone();
            tokio::spawn(async move {
                let request: ThreadCreate = serde_json::from_value(
                    json!({ "topic": { "key": "release-1.4" }, "key": "phase-3", "title": "Phase 3" }),
                )
                .unwrap();
                handle.create_thread("claude-code".to_owned(), request).await
            })
        })
        .collect();
    let mut created = 0;
    for thread in threads {
        created += usize::from(thread.await.unwrap().unwrap().was_created());
    }
    assert_eq!(created, 1);
    hub.stop().await;
}

#[tokio::test]
async fn a_conversation_thread_takes_over_the_implicit_one_once() {
    let hub = TestHub::start();
    let first = hub.create(blocking_prompt()).await;
    let implicit = hub.handle.thread(first.thread_id.clone()).await.unwrap();
    assert!(implicit.implicit);
    let request: ThreadCreate = serde_json::from_value(json!({
        "topic": { "id": first.topic_id }, "key": "gentbox-session", "title": "Gentbox session",
        "kind": "conversation", "conversation": { "id": "conversation-1" }
    }))
    .unwrap();
    let Upserted::Created(thread) = hub
        .handle
        .create_thread("claude-code".to_owned(), request.clone())
        .await
        .unwrap()
    else {
        panic!("expected a new thread");
    };
    assert!(thread.default_for.is_some());
    let moved = hub.handle.message(first.id.clone()).await.unwrap();
    assert_eq!(moved.thread_id, thread.id);
    assert!(matches!(
        hub.handle.thread(implicit.id).await,
        Err(HubError::ThreadNotFound(_))
    ));
    // Later default messages land in it; the same create is idempotent.
    let later = hub
        .create(json!({ "type": "task_completed", "title": "Phase 3 is done" }))
        .await;
    assert_eq!(later.thread_id, thread.id);
    let again = hub
        .handle
        .create_thread("claude-code".to_owned(), request)
        .await
        .unwrap();
    assert!(!again.was_created());
    hub.stop().await;
}

#[tokio::test]
async fn closing_withdraws_open_messages_only_when_asked_and_only_the_requesters() {
    let hub = TestHub::start();
    let mine = hub
        .create(json!({ "type": "decision_required", "title": "Ship 1.4?", "topic": { "key": "release-1.4" } }))
        .await;
    let theirs = hub
        .create(json!({
            "type": "decision_required", "title": "Tag 1.4?", "topic": { "key": "release-1.4" },
            "source": { "agent": "codex" }
        }))
        .await;
    let closed = hub
        .handle
        .close_topic(
            mine.topic_id.clone(),
            CloseRequest::default(),
            Some(Requester::agent("claude-code")),
        )
        .await
        .unwrap();
    assert_eq!(closed.status, GroupStatus::Closed);
    assert_eq!(hub.status_of(&mine.id).await, ResponseStatus::Pending);

    hub.handle
        .reopen_topic(mine.topic_id.clone())
        .await
        .unwrap();
    hub.handle
        .close_topic(
            mine.topic_id.clone(),
            CloseRequest {
                rescind_open_messages: true,
                reason: None,
            },
            Some(Requester::agent("claude-code")),
        )
        .await
        .unwrap();
    let response = hub.poll(&mine.id, std::time::Duration::ZERO).await;
    assert_eq!(response.status, ResponseStatus::Rescinded);
    assert_eq!(
        response.rescind_reason,
        Some(gentbox_core::rescind::RescindReason::Obsolete)
    );
    assert_eq!(hub.status_of(&theirs.id).await, ResponseStatus::Pending);

    // The UI (no requester) withdraws everything.
    hub.handle
        .rescind_topic(
            mine.topic_id.clone(),
            gentbox_core::rescind::RescindReason::Obsolete,
            None,
        )
        .await
        .unwrap();
    assert_eq!(hub.status_of(&theirs.id).await, ResponseStatus::Rescinded);
    hub.stop().await;
}

#[tokio::test]
async fn a_message_to_a_closed_thread_reopens_it_and_its_topic() {
    let hub = TestHub::start();
    let first = hub
        .create(json!({ "type": "generic", "title": "Started phase 3", "topic": { "key": "gentbox" }, "thread": { "key": "phase-3" } }))
        .await;
    hub.handle
        .close_topic(first.topic_id.clone(), CloseRequest::default(), None)
        .await
        .unwrap();
    assert_eq!(
        hub.handle
            .thread(first.thread_id.clone())
            .await
            .unwrap()
            .status,
        GroupStatus::Closed
    );
    let second = hub
        .create(json!({ "type": "generic", "title": "Phase 3 continues", "thread": { "id": first.thread_id } }))
        .await;
    assert_eq!(second.thread_id, first.thread_id);
    assert_eq!(
        hub.handle.thread(first.thread_id).await.unwrap().status,
        GroupStatus::Open
    );
    assert_eq!(
        hub.handle.topic(first.topic_id).await.unwrap().status,
        GroupStatus::Open
    );
    hub.stop().await;
}

#[tokio::test]
async fn implicit_groups_refuse_patches_and_titles_are_guarded() {
    let hub = TestHub::start();
    let first = hub.create(blocking_prompt()).await;
    let patch = TopicUpdate {
        title: Some("Mine now".to_owned()),
        ..TopicUpdate::default()
    };
    assert!(matches!(
        hub.handle.update_topic(first.topic_id.clone(), patch).await,
        Err(HubError::ImplicitGroup)
    ));
    let patch = ThreadUpdate {
        title: Some("Mine now".to_owned()),
        ..ThreadUpdate::default()
    };
    assert!(matches!(
        hub.handle
            .update_thread(first.thread_id.clone(), patch)
            .await,
        Err(HubError::ImplicitGroup)
    ));

    let topic = hub
        .handle
        .create_topic("claude-code".to_owned(), topic_create("release-1.4"))
        .await
        .unwrap()
        .into_inner();
    let flagged = TopicUpdate {
        title: Some("rm -rf build".to_owned()),
        ..TopicUpdate::default()
    };
    assert!(matches!(
        hub.handle.update_topic(topic.id.clone(), flagged).await,
        Err(HubError::Grouping(_))
    ));
    let cleared = TopicUpdate {
        summary: Some(None),
        title: Some("Release 1.4 and docs".to_owned()),
        ..TopicUpdate::default()
    };
    let updated = hub.handle.update_topic(topic.id, cleared).await.unwrap();
    assert_eq!(updated.title, "Release 1.4 and docs");
    assert_eq!(updated.summary, None);
    let flagged_create: TopicCreate =
        serde_json::from_value(json!({ "key": "bad", "title": "npm publish --tag next" })).unwrap();
    assert!(matches!(
        hub.handle
            .create_topic("claude-code".to_owned(), flagged_create)
            .await,
        Err(HubError::Grouping(_))
    ));
    hub.stop().await;
}

#[tokio::test]
async fn moving_a_thread_into_a_topic_with_its_key_is_a_conflict() {
    let hub = TestHub::start();
    let a = hub
        .create(json!({ "type": "generic", "title": "One", "topic": { "key": "a" }, "thread": { "key": "phase-3" } }))
        .await;
    let b = hub
        .create(json!({ "type": "generic", "title": "Two", "topic": { "key": "b" }, "thread": { "key": "phase-3" } }))
        .await;
    let request: ThreadMove =
        serde_json::from_value(json!({ "topic": { "id": b.topic_id } })).unwrap();
    assert!(matches!(
        hub.handle
            .move_thread(
                a.thread_id.clone(),
                request,
                Requester::agent("claude-code")
            )
            .await,
        Err(HubError::Conflict(_))
    ));
    let request: ThreadMove = serde_json::from_value(json!({ "topic": { "key": "c" } })).unwrap();
    let moved = hub
        .handle
        .move_thread(a.thread_id, request, Requester::agent("claude-code"))
        .await
        .unwrap();
    assert_ne!(moved.topic_id, a.topic_id);
    let message = hub.handle.message(a.id).await.unwrap();
    assert_eq!(message.topic_id, moved.topic_id);
    hub.stop().await;
}
