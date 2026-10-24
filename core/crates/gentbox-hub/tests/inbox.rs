//! What the Mac inbox reads through FFI: the snapshot in one actor turn, and
//! the moves you make there, which skip the agents' owner check.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use gentbox_core::message::ActionKind;
use gentbox_core::topic::{TopicCreate, TopicRef};
use gentbox_hub::{HubError, MessageThreadMove, Requester, ThreadMove};
use serde_json::json;
use support::{TestHub, blocking_prompt, message};

#[tokio::test]
async fn the_snapshot_lists_live_unsnoozed_messages_with_their_groups_and_buttons() {
    let hub = TestHub::start();
    let prompt = hub.create(blocking_prompt()).await;
    let done = hub
        .create(json!({ "type": "task_completed", "title": "Ingest tests pass" }))
        .await;
    let snoozed = hub
        .create(json!({ "type": "generic", "title": "Nightly build log is ready" }))
        .await;
    hub.handle
        .snooze(snoozed.id.clone(), Some(30))
        .await
        .unwrap();
    hub.handle.dismiss(done.id.clone()).await.unwrap();

    let snapshot = hub.handle.inbox_snapshot().await.unwrap();
    let ids: Vec<_> = snapshot
        .entries
        .iter()
        .map(|entry| &entry.message.id)
        .collect();
    assert_eq!(ids, vec![&prompt.id]);
    assert!(
        snapshot
            .topics
            .iter()
            .any(|topic| topic.id == prompt.topic_id)
    );
    assert!(
        snapshot
            .threads
            .iter()
            .any(|thread| thread.id == prompt.thread_id)
    );

    // The buttons follow the waiter (architecture §9.1): answers only while a
    // sender is waiting.
    hub.handle.begin_wait(prompt.id.clone()).await.unwrap();
    let snapshot = hub.handle.inbox_snapshot().await.unwrap();
    let entry = &snapshot.entries[0];
    assert!(entry.has_live_waiter);
    assert!(
        entry
            .offered_actions
            .iter()
            .any(|action| action.kind == ActionKind::Approve)
    );
    hub.stop().await;
}

#[tokio::test]
async fn you_move_another_agents_thread_and_message_where_the_agent_may_not() {
    let hub = TestHub::start();
    let first = hub.create(blocking_prompt()).await;
    let topic: TopicCreate =
        serde_json::from_value(json!({ "key": "infra", "title": "Infrastructure" })).unwrap();
    let infra = hub
        .handle
        .create_topic("codex".to_owned(), topic)
        .await
        .unwrap()
        .into_inner();
    let move_to_infra = ThreadMove {
        topic: TopicRef::Id(gentbox_core::topic::TopicIdRef {
            id: infra.id.clone(),
        }),
    };

    let refused = hub
        .handle
        .move_thread(
            first.thread_id.clone(),
            move_to_infra.clone(),
            Requester::agent("codex"),
        )
        .await;
    assert!(matches!(refused, Err(HubError::NotOwner)));

    let moved = hub
        .handle
        .move_thread_for_user(first.thread_id.clone(), move_to_infra)
        .await
        .unwrap();
    assert_eq!(moved.topic_id, infra.id);
    let message_now = hub.handle.message(first.id.clone()).await.unwrap();
    assert_eq!(message_now.topic_id, infra.id);

    // Back to its default resolution: the conversation's thread in General.
    let other = hub
        .handle
        .ingest(message(json!({
            "source": { "agent": "codex" },
            "conversation": { "id": "codex-1" },
            "title": "Staging certificates rotated"
        })))
        .await
        .unwrap()
        .response;
    let regrouped = hub
        .handle
        .move_message_for_user(
            other.id.clone(),
            MessageThreadMove {
                thread: None,
                topic: Some(TopicRef::Id(gentbox_core::topic::TopicIdRef {
                    id: infra.id.clone(),
                })),
            },
        )
        .await
        .unwrap();
    assert_eq!(regrouped.topic_id, infra.id);
    hub.stop().await;
}
