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
async fn a_group_with_open_messages_cannot_be_closed() {
    let hub = TestHub::start();
    let mine = hub
        .create(json!({ "type": "decision_required", "title": "Ship 1.4?", "topic": { "key": "release-1.4" }, "thread": { "key": "rc" } }))
        .await;
    let theirs = hub
        .create(json!({
            "type": "decision_required", "title": "Tag 1.4?", "topic": { "key": "release-1.4" },
            "thread": { "key": "rc" }, "source": { "agent": "codex" }
        }))
        .await;
    for refused in [
        hub.handle
            .close_topic(mine.topic_id.clone(), CloseRequest::default(), None)
            .await
            .map(|_| ()),
        hub.handle
            .close_thread(mine.thread_id.clone(), CloseRequest::default(), None)
            .await
            .map(|_| ()),
    ] {
        assert!(matches!(
            refused,
            Err(HubError::HasOpenMessages { count: 2 })
        ));
    }
    assert_eq!(
        hub.handle
            .topic(mine.topic_id.clone())
            .await
            .unwrap()
            .status,
        GroupStatus::Open
    );
    // Snoozed and acknowledged messages are still open.
    hub.handle
        .snooze(theirs.id.clone(), Some(30))
        .await
        .unwrap();
    hub.handle.acknowledge(mine.id.clone()).await.unwrap();
    let refused = hub
        .handle
        .close_thread(mine.thread_id.clone(), CloseRequest::default(), None)
        .await;
    assert!(matches!(
        refused,
        Err(HubError::HasOpenMessages { count: 2 })
    ));
    hub.stop().await;
}

#[tokio::test]
async fn an_agent_close_withdraws_only_its_own_and_is_refused_when_others_stay_open() {
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
    let withdraw_mine = CloseRequest {
        rescind_open_messages: true,
        reason: None,
    };
    // Codex's question would stay open: nothing is withdrawn, nothing closes.
    let refused = hub
        .handle
        .close_topic(
            mine.topic_id.clone(),
            withdraw_mine.clone(),
            Some(Requester::agent("claude-code")),
        )
        .await;
    assert!(matches!(
        refused,
        Err(HubError::HasOpenMessages { count: 1 })
    ));
    assert_eq!(hub.status_of(&mine.id).await, ResponseStatus::Pending);

    hub.handle.dismiss(theirs.id.clone()).await.unwrap();
    let closed = hub
        .handle
        .close_topic(
            mine.topic_id.clone(),
            withdraw_mine,
            Some(Requester::agent("claude-code")),
        )
        .await
        .unwrap();
    assert_eq!(closed.status, GroupStatus::Closed);
    let response = hub.poll(&mine.id, std::time::Duration::ZERO).await;
    assert_eq!(response.status, ResponseStatus::Rescinded);
    assert_eq!(
        response.rescind_reason,
        Some(gentbox_core::rescind::RescindReason::Obsolete)
    );
    hub.stop().await;
}

#[tokio::test]
async fn the_general_topic_cannot_be_closed_but_its_threads_can() {
    let hub = TestHub::start();
    let note = hub
        .create(json!({ "type": "generic", "title": "Nightly build log is ready" }))
        .await;
    let general = hub.handle.topic(note.topic_id.clone()).await.unwrap();
    assert!(general.implicit);
    hub.handle.dismiss(note.id.clone()).await.unwrap();
    let refused = hub
        .handle
        .close_topic(note.topic_id.clone(), CloseRequest::default(), None)
        .await;
    assert!(matches!(refused, Err(HubError::GeneralTopic)));
    let thread = hub
        .handle
        .close_thread(note.thread_id.clone(), CloseRequest::default(), None)
        .await
        .unwrap();
    assert_eq!(thread.status, GroupStatus::Closed);
    hub.stop().await;
}

#[tokio::test]
async fn a_message_to_a_closed_thread_reopens_it_and_its_topic() {
    let hub = TestHub::start();
    let first = hub
        .create(json!({ "type": "generic", "title": "Started phase 3", "topic": { "key": "gentbox" }, "thread": { "key": "phase-3" } }))
        .await;
    // Only a group with nothing open closes.
    hub.handle.dismiss(first.id.clone()).await.unwrap();
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

#[tokio::test]
async fn moving_a_thread_with_open_items_into_a_closed_topic_reopens_it() {
    let hub = TestHub::start();
    let parked = hub
        .create(json!({ "type": "generic", "title": "Release notes drafted", "topic": { "key": "archive", "title": "Archive" } }))
        .await;
    hub.handle.dismiss(parked.id.clone()).await.unwrap();
    hub.handle
        .close_topic(parked.topic_id.clone(), CloseRequest::default(), None)
        .await
        .unwrap();
    let prompt = hub.create(blocking_prompt()).await;
    let into_archive = ThreadMove {
        topic: gentbox_core::topic::TopicRef::Id(gentbox_core::topic::TopicIdRef {
            id: parked.topic_id.clone(),
        }),
    };
    hub.handle
        .move_thread_for_user(prompt.thread_id.clone(), into_archive)
        .await
        .unwrap();
    // An open item never sits in a closed group, where the inbox would hide it.
    let topic = hub.handle.topic(parked.topic_id.clone()).await.unwrap();
    assert_eq!(topic.status, GroupStatus::Open);
    hub.stop().await;
}

#[tokio::test]
async fn nobody_withdraws_every_agents_messages_without_naming_who_asks() {
    let hub = TestHub::start();
    let prompt = hub
        .create(json!({ "type": "decision_required", "title": "Ship 1.4?", "topic": { "key": "release-1.4" } }))
        .await;
    let withdraw = CloseRequest {
        rescind_open_messages: true,
        reason: None,
    };
    let refused = hub
        .handle
        .close_topic(prompt.topic_id.clone(), withdraw, None)
        .await;
    assert!(matches!(refused, Err(HubError::NoRequester)));
    let refused = hub
        .handle
        .rescind_topic(
            prompt.topic_id.clone(),
            gentbox_core::rescind::RescindReason::Obsolete,
            None,
        )
        .await;
    assert!(matches!(refused, Err(HubError::NoRequester)));
    assert_eq!(hub.status_of(&prompt.id).await, ResponseStatus::Pending);
    hub.stop().await;
}
