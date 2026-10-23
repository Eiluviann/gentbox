//! Fixtures shared by the store tests.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use gentbox_core::classify::{BuiltInRules, RuleSet};
use gentbox_core::ids::{MessageId, ThreadId, TopicId};
use gentbox_core::lifecycle::{Lifecycle, LifecycleState, ResolutionReason};
use gentbox_core::message::{Blocking, Message, MessageType};
use gentbox_core::prepare::{IngestSettings, prepare_message};
use gentbox_core::topic::{CreatedBy, DefaultFor, GroupStatus, Thread, Topic};
use gentbox_store::{Repository, Store};
use serde_json::{Value, json};
use tempfile::TempDir;
use time::macros::datetime;
use time::{Duration, OffsetDateTime};

pub const NOW: OffsetDateTime = datetime!(2026-10-24 12:00:00 UTC);

pub fn memory_store() -> Store {
    Store::open_in_memory().unwrap()
}

/// A store on a file in a fresh temp dir; keep the dir alive with it.
pub fn file_store() -> (TempDir, Store) {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path().join("gentbox.db")).unwrap();
    (directory, store)
}

pub fn topic_id(name: &str) -> TopicId {
    TopicId::new(format!("topic-{name}"))
}

pub fn thread_id(name: &str) -> ThreadId {
    ThreadId::new(format!("thread-{name}"))
}

pub fn message_id(name: &str) -> MessageId {
    MessageId::new(format!("message-{name}"))
}

pub fn created_by(agent: &str) -> CreatedBy {
    CreatedBy {
        agent: agent.to_owned(),
        conversation_id: None,
    }
}

/// An explicit, open topic with a key, created at `NOW`.
pub fn topic(name: &str) -> Topic {
    Topic {
        id: topic_id(name),
        key: Some(format!("key:{name}")),
        title: format!("Topic {name}"),
        summary: None,
        kind: None,
        style: None,
        implicit: false,
        status: GroupStatus::Open,
        muted: false,
        marked_read_at: None,
        created_by: created_by("claude-code"),
        created_at: NOW,
        updated_at: NOW,
        closed_at: None,
    }
}

/// An agent's implicit General topic.
pub fn general_topic(name: &str, agent: &str) -> Topic {
    Topic {
        key: None,
        title: "General".to_owned(),
        kind: Some("general".to_owned()),
        implicit: true,
        created_by: created_by(agent),
        ..topic(name)
    }
}

/// An explicit, open thread with a key in `topic`, created at `NOW`.
pub fn thread(name: &str, topic: &str) -> Thread {
    Thread {
        id: thread_id(name),
        topic_id: topic_id(topic),
        key: Some(format!("key:{name}")),
        title: format!("Thread {name}"),
        summary: None,
        kind: None,
        implicit: false,
        status: GroupStatus::Open,
        closed_by_topic: false,
        muted: false,
        marked_read_at: None,
        default_for: None,
        created_by: created_by("claude-code"),
        created_at: NOW,
        updated_at: NOW,
        closed_at: None,
    }
}

/// A conversation's implicit default thread in `topic`.
pub fn implicit_thread(name: &str, topic: &str, conversation_id: &str) -> Thread {
    Thread {
        key: None,
        kind: Some("conversation".to_owned()),
        implicit: true,
        default_for: Some(DefaultFor {
            agent: "claude-code".to_owned(),
            conversation_id: conversation_id.to_owned(),
        }),
        ..thread(name, topic)
    }
}

/// Inserts topic `name` and one thread `name` in it.
pub fn seed_group(repository: &Repository<'_>, name: &str) {
    repository.insert_topic(&topic(name)).unwrap();
    repository.insert_thread(&thread(name, name)).unwrap();
}

/// A message built by the core pipeline, placed in `thread` of `topic`,
/// received at `NOW`.
pub fn message(name: &str, topic: &str, thread: &str, extra: Value) -> Message {
    let mut fields = json!({
        "version": 1,
        "source": { "agent": "claude-code", "displayName": "Claude Code" },
        "conversation": { "id": "conversation-1", "cwd": "/Users/me/Developer/gentbox" },
        "title": "Ingest tests pass",
    });
    if let (Some(target), Some(extra)) = (fields.as_object_mut(), extra.as_object()) {
        for (key, value) in extra {
            target.insert(key.clone(), value.clone());
        }
    }
    let input = serde_json::from_value(fields).unwrap();
    prepare_message(
        input,
        &RuleSet::empty(),
        BuiltInRules::load(),
        IngestSettings::default(),
    )
    .unwrap()
    .into_message(message_id(name), NOW, topic_id(topic), thread_id(thread))
}

/// A message with declared type, urgency and blocking level.
pub fn classified(
    name: &str,
    topic: &str,
    thread: &str,
    message_type: MessageType,
    urgent: bool,
    blocking: Blocking,
) -> Message {
    message(
        name,
        topic,
        thread,
        json!({
            "type": message_type.as_str(),
            "urgent": urgent,
            "blocking": blocking.as_str(),
        }),
    )
}

/// A plain `decision_required` message in `thread`.
pub fn plain(name: &str, topic: &str, thread: &str) -> Message {
    classified(
        name,
        topic,
        thread,
        MessageType::DecisionRequired,
        false,
        Blocking::Blocking,
    )
}

/// The same message, closed in `state` with the matching resolution.
pub fn closed(mut message: Message, state: LifecycleState) -> Message {
    message.lifecycle = Lifecycle {
        state,
        snoozed_until: None,
        deferred_at: None,
        resolution: Some(match state {
            LifecycleState::Answered => ResolutionReason::Answered,
            LifecycleState::Dismissed => ResolutionReason::Dismissed,
            LifecycleState::Rescinded => {
                ResolutionReason::Rescinded(gentbox_core::rescind::RescindReason::Obsolete)
            }
            LifecycleState::Expired => ResolutionReason::Expired,
            other => panic!("{other:?} is not a closed state"),
        }),
        answer: None,
    };
    message
}

pub fn days_before(instant: OffsetDateTime, days: i64) -> OffsetDateTime {
    instant - Duration::days(days)
}
