//! What the Mac inbox reads through FFI: the snapshot in one actor turn, and
//! the moves you make there, which skip the agents' owner check.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use gentbox_core::lifecycle::LifecycleState;
use gentbox_core::message::ActionKind;
use gentbox_core::response::ResponseStatus;
use gentbox_core::topic::{GroupStatus, TopicCreate, TopicRef};
use gentbox_hub::{HubError, HubEvent, MessageThreadMove, Requester, ThreadMove};
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

#[tokio::test]
async fn closing_from_the_mac_dismisses_and_never_withdraws() {
    let hub = TestHub::start();
    // One thread, three conversations: in one conversation the second
    // blocking message would fold into the waiting prompt (architecture §7).
    let in_thread = |conversation: &str, fields: serde_json::Value| {
        let mut fields = fields;
        support::merge(
            &mut fields,
            json!({
                "conversation": { "id": conversation },
                "topic": { "key": "gentbox", "title": "Gentbox" },
                "thread": { "key": "phase-3", "title": "Phase 3" }
            }),
        );
        fields
    };
    let prompt = hub
        .create(in_thread("conversation-1", blocking_prompt()))
        .await;
    let action = hub
        .create(in_thread(
            "conversation-2",
            support::action_without_waiter(),
        ))
        .await;
    let note = hub
        .create(in_thread(
            "conversation-3",
            json!({ "type": "generic", "title": "Nightly build log is ready" }),
        ))
        .await;
    assert_eq!(prompt.thread_id, action.thread_id);
    hub.recorder.clear();

    hub.handle
        .close_thread_for_user(prompt.thread_id.clone(), true)
        .await
        .unwrap();

    // The waiting sender hears `dismissed`: "Withdrawn" is the agent's word.
    assert_eq!(hub.status_of(&prompt.id).await, ResponseStatus::Dismissed);
    let note_now = hub.handle.message(note.id.clone()).await.unwrap();
    assert_eq!(note_now.lifecycle.state, LifecycleState::Dismissed);
    // An action_required message closes only with Done.
    let action_now = hub.handle.message(action.id.clone()).await.unwrap();
    assert!(action_now.lifecycle.state.is_live());
    let events = hub.recorder.events();
    assert_eq!(
        support::count(&events, |event| matches!(event, HubEvent::Withdrawn(_))),
        0
    );
    let thread = hub.handle.thread(prompt.thread_id.clone()).await.unwrap();
    assert_eq!(thread.status, GroupStatus::Closed);

    // Closing a topic without dismissing leaves its messages open.
    let other = hub
        .create(json!({
            "conversation": { "id": "conversation-4" },
            "type": "task_completed", "title": "Docs build"
        }))
        .await;
    hub.handle
        .close_topic_for_user(other.topic_id.clone(), false)
        .await
        .unwrap();
    let other_now = hub.handle.message(other.id.clone()).await.unwrap();
    assert!(other_now.lifecycle.state.is_live());
    hub.stop().await;
}
