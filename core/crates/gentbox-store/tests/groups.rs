//! Topics and threads (ADRs 0008, 0009): upserts, default threads, takeover,
//! moves, mark read, close and reopen.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use gentbox_core::lifecycle::{LifecycleEvent, LifecycleState, apply_event};
use gentbox_core::resolve::Takeover;
use gentbox_core::topic::{GroupStatus, ThreadRecord, TopicColor, TopicRecord, TopicStyle};
use gentbox_store::{Conflict, EntityKind, MessageFilter, Store, StoreError, Upserted};
use support::{
    NOW, closed, created_by, explicit_default, general_topic, implicit_thread, memory_store,
    message_id, plain, thread, thread_id, topic, topic_id,
};
use time::Duration;

fn later(minutes: i64) -> time::OffsetDateTime {
    NOW + Duration::minutes(minutes)
}

fn store_with_topics(names: &[&str]) -> Store {
    let store = memory_store();
    for name in names {
        store.repository().insert_topic(&topic(name)).unwrap();
    }
    store
}

// --- topics ------------------------------------------------------------------

#[test]
fn a_topic_round_trips_with_every_field() {
    let store = memory_store();
    let mut stored = topic("full");
    stored.summary = Some("The big undertaking".to_owned());
    stored.kind = Some("project".to_owned());
    stored.style = Some(TopicStyle {
        symbol: Some("hammer".to_owned()),
        color: Some(TopicColor::Teal),
    });
    stored.muted = true;
    stored.marked_read_at = Some(later(3));
    stored.status = GroupStatus::Closed;
    stored.closed_at = Some(later(4));
    stored.created_by.conversation_id = Some("conversation-1".to_owned());
    stored.updated_at = later(5);
    store.repository().insert_topic(&stored).unwrap();
    assert_eq!(store.repository().topic(&stored.id).unwrap(), stored);
}

#[test]
fn every_topic_colour_and_an_empty_style_round_trip() {
    let store = memory_store();
    let colours = [
        TopicColor::Blue,
        TopicColor::Purple,
        TopicColor::Pink,
        TopicColor::Red,
        TopicColor::Orange,
        TopicColor::Yellow,
        TopicColor::Green,
        TopicColor::Teal,
        TopicColor::Indigo,
        TopicColor::Gray,
    ];
    for (index, color) in colours.into_iter().enumerate() {
        let mut stored = topic(&format!("c{index}"));
        stored.style = Some(TopicStyle {
            symbol: None,
            color: Some(color),
        });
        store.repository().insert_topic(&stored).unwrap();
        assert_eq!(store.repository().topic(&stored.id).unwrap(), stored);
    }
    let mut empty_style = topic("empty-style");
    empty_style.style = Some(TopicStyle::default());
    store.repository().insert_topic(&empty_style).unwrap();
    assert_eq!(
        store.repository().topic(&empty_style.id).unwrap().style,
        Some(TopicStyle::default())
    );
}

#[test]
fn topics_upsert_by_key() {
    let store = memory_store();
    let repository = store.repository();
    let first = topic("one");
    let created = repository.upsert_topic(&first).unwrap();
    assert_eq!(created, Upserted::Created(first.clone()));
    assert!(created.was_created());

    let mut again = topic("another-id");
    again.key = first.key.clone();
    again.title = "A different title".to_owned();
    let existing = repository.upsert_topic(&again).unwrap();
    assert_eq!(existing, Upserted::Existing(first.clone()));
    assert_eq!(existing.into_inner().title, "Topic one");
    assert_eq!(repository.list_topics(None).unwrap(), [first]);
}

#[test]
fn a_topic_key_is_unique_per_hub_across_agents() {
    let store = memory_store();
    let repository = store.repository();
    repository.insert_topic(&topic("one")).unwrap();
    let mut other_agent = topic("two");
    other_agent.key = topic("one").key;
    other_agent.created_by = created_by("codex");
    let error = repository.insert_topic(&other_agent).unwrap_err();
    assert!(matches!(error, StoreError::Conflict(Conflict::TopicKey(key)) if key == "key:one"));
    assert!(matches!(
        repository.upsert_topic(&other_agent).unwrap(),
        Upserted::Existing(existing) if existing.id == topic_id("one")
    ));
}

#[test]
fn a_duplicate_topic_id_is_a_conflict() {
    let store = store_with_topics(&["one"]);
    let mut duplicate = topic("one");
    duplicate.key = Some("key:fresh".to_owned());
    assert!(matches!(
        store.repository().insert_topic(&duplicate).unwrap_err(),
        StoreError::Conflict(Conflict::TopicId(_))
    ));
}

#[test]
fn each_agent_has_one_implicit_general_topic() {
    let store = memory_store();
    let repository = store.repository();
    assert!(
        repository
            .find_general_topic("claude-code")
            .unwrap()
            .is_none()
    );
    repository
        .insert_topic(&general_topic("g1", "claude-code"))
        .unwrap();
    repository
        .insert_topic(&general_topic("g2", "codex"))
        .unwrap();
    assert_eq!(
        repository
            .find_general_topic("claude-code")
            .unwrap()
            .map(|topic| topic.id),
        Some(topic_id("g1"))
    );
    assert_eq!(
        repository
            .find_general_topic("codex")
            .unwrap()
            .map(|topic| topic.id),
        Some(topic_id("g2"))
    );
    assert!(matches!(
        repository.insert_topic(&general_topic("g3", "claude-code")).unwrap_err(),
        StoreError::Conflict(Conflict::GeneralTopic(agent)) if agent == "claude-code"
    ));
}

#[test]
fn a_missing_topic_is_not_found() {
    let store = memory_store();
    let error = store.repository().topic(&topic_id("ghost")).unwrap_err();
    assert!(matches!(
        error,
        StoreError::NotFound {
            entity: EntityKind::Topic,
            ..
        }
    ));
    assert!(
        store
            .repository()
            .find_topic_by_key("nothing")
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        store
            .repository()
            .update_topic(&topic("ghost"))
            .unwrap_err(),
        StoreError::NotFound {
            entity: EntityKind::Topic,
            ..
        }
    ));
}

#[test]
fn updating_a_topic_saves_what_you_and_agents_may_change() {
    let store = store_with_topics(&["one"]);
    let repository = store.repository();
    let mut changed = topic("one");
    changed.title = "Renamed".to_owned();
    changed.summary = Some("Now with a summary".to_owned());
    changed.kind = Some("category".to_owned());
    changed.muted = true;
    changed.updated_at = later(1);
    // Identity and origin never change through an update.
    changed.key = Some("key:ignored".to_owned());
    changed.created_by = created_by("somebody-else");
    repository.update_topic(&changed).unwrap();

    let stored = repository.topic(&topic_id("one")).unwrap();
    assert_eq!(stored.title, "Renamed");
    assert!(stored.muted);
    assert_eq!(stored.key.as_deref(), Some("key:one"));
    assert_eq!(stored.created_by.agent, "claude-code");
}

#[test]
fn topics_list_by_status() {
    let store = store_with_topics(&["a", "b", "c"]);
    let repository = store.repository();
    repository.close_topic(&topic_id("b"), later(1)).unwrap();
    let ids = |status| -> Vec<_> {
        repository
            .list_topics(status)
            .unwrap()
            .into_iter()
            .map(|topic| topic.id)
            .collect()
    };
    assert_eq!(ids(None), [topic_id("a"), topic_id("b"), topic_id("c")]);
    assert_eq!(ids(Some(GroupStatus::Open)), [topic_id("a"), topic_id("c")]);
    assert_eq!(ids(Some(GroupStatus::Closed)), [topic_id("b")]);
}

// --- threads -------------------------------------------------------------------

#[test]
fn a_thread_round_trips_with_every_field_and_its_default_conversation() {
    let store = store_with_topics(&["one"]);
    let mut stored = thread("full", "one");
    stored.summary = Some("Phase 3".to_owned());
    stored.kind = Some("phase".to_owned());
    stored.muted = true;
    stored.marked_read_at = Some(later(1));
    stored.status = GroupStatus::Closed;
    stored.closed_by_topic = true;
    stored.closed_at = Some(later(2));
    stored.created_by.conversation_id = Some("conversation-1".to_owned());
    stored.default_for = Some(gentbox_core::topic::DefaultFor {
        agent: "claude-code".to_owned(),
        conversation_id: "conversation-1".to_owned(),
    });
    store.repository().insert_thread(&stored).unwrap();
    assert_eq!(store.repository().thread(&stored.id).unwrap(), stored);
}

#[test]
fn threads_upsert_by_key_within_their_topic() {
    let store = store_with_topics(&["one", "two"]);
    let repository = store.repository();
    let first = thread("a", "one");
    assert!(repository.upsert_thread(&first).unwrap().was_created());

    let mut repeat = thread("b", "one");
    repeat.key = first.key.clone();
    repeat.title = "Other title".to_owned();
    assert_eq!(
        repository.upsert_thread(&repeat).unwrap(),
        Upserted::Existing(first.clone())
    );

    // The same key under another topic is a different thread.
    let mut elsewhere = thread("c", "two");
    elsewhere.key = first.key.clone();
    assert!(repository.upsert_thread(&elsewhere).unwrap().was_created());

    assert!(matches!(
        repository.insert_thread(&repeat).unwrap_err(),
        StoreError::Conflict(Conflict::ThreadKey { key, .. }) if key == "key:a"
    ));
    assert_eq!(
        repository
            .find_thread_by_key(&topic_id("two"), "key:a")
            .unwrap()
            .map(|thread| thread.id),
        Some(thread_id("c"))
    );
}

#[test]
fn implicit_threads_without_keys_coexist() {
    let store = store_with_topics(&["one"]);
    let repository = store.repository();
    repository
        .insert_thread(&implicit_thread("i1", "one", "conversation-1"))
        .unwrap();
    repository
        .insert_thread(&implicit_thread("i2", "one", "conversation-2"))
        .unwrap();
    assert_eq!(
        repository
            .list_threads(Some(&topic_id("one")), None)
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn a_thread_needs_an_existing_topic() {
    let store = memory_store();
    assert!(matches!(
        store
            .repository()
            .insert_thread(&thread("a", "ghost"))
            .unwrap_err(),
        StoreError::NotFound {
            entity: EntityKind::Topic,
            ..
        }
    ));
    assert!(matches!(
        store.repository().thread(&thread_id("ghost")).unwrap_err(),
        StoreError::NotFound {
            entity: EntityKind::Thread,
            ..
        }
    ));
}

#[test]
fn a_duplicate_thread_id_is_a_conflict() {
    let store = store_with_topics(&["one"]);
    store
        .repository()
        .insert_thread(&thread("a", "one"))
        .unwrap();
    let mut duplicate = thread("a", "one");
    duplicate.key = Some("key:fresh".to_owned());
    assert!(matches!(
        store.repository().insert_thread(&duplicate).unwrap_err(),
        StoreError::Conflict(Conflict::ThreadId(_))
    ));
}

#[test]
fn a_conversation_has_one_default_thread_per_topic() {
    let store = store_with_topics(&["one", "two"]);
    let repository = store.repository();
    repository
        .insert_thread(&implicit_thread("one-default", "one", "conversation-1"))
        .unwrap();
    let error = repository
        .insert_thread(&implicit_thread("second-default", "one", "conversation-1"))
        .unwrap_err();
    assert!(matches!(
        error,
        StoreError::Conflict(Conflict::DefaultThread { .. })
    ));
    // A second topic gets its own default thread for the same conversation.
    repository
        .insert_thread(&implicit_thread("two-default", "two", "conversation-1"))
        .unwrap();

    let link = repository
        .default_thread_link("claude-code", "conversation-1", &topic_id("two"))
        .unwrap()
        .unwrap();
    assert_eq!(link.thread_id, thread_id("two-default"));
    assert_eq!(link.last_used, NOW);
    assert!(
        repository
            .default_thread_link("claude-code", "conversation-9", &topic_id("two"))
            .unwrap()
            .is_none()
    );
    assert_eq!(repository.default_links().unwrap().len(), 2);
}

#[test]
fn touching_a_default_thread_moves_its_last_used_time() {
    let store = store_with_topics(&["one"]);
    let repository = store.repository();
    repository
        .insert_thread(&implicit_thread("d", "one", "conversation-1"))
        .unwrap();
    repository
        .touch_default_thread("claude-code", "conversation-1", &topic_id("one"), later(9))
        .unwrap();
    let link = repository
        .default_thread_link("claude-code", "conversation-1", &topic_id("one"))
        .unwrap()
        .unwrap();
    assert_eq!(link.last_used, later(9));
    assert!(matches!(
        repository
            .touch_default_thread("claude-code", "nobody", &topic_id("one"), NOW)
            .unwrap_err(),
        StoreError::NotFound {
            entity: EntityKind::Thread,
            ..
        }
    ));
}

#[test]
fn a_thread_spans_two_conversations() {
    let store = store_with_topics(&["one"]);
    let repository = store.repository();
    repository.insert_thread(&thread("shared", "one")).unwrap();
    for (name, conversation) in [
        ("a", "conversation-1"),
        ("b", "conversation-2"),
        ("c", "conversation-2"),
    ] {
        let mut message = plain(name, "one", "shared");
        message.conversation.id = conversation.to_owned();
        repository.insert_message(&message, NOW).unwrap();
    }
    let record = repository.thread_record(&thread_id("shared")).unwrap();
    assert_eq!(record.conversations, Some(2));
    assert_eq!(record.open_messages, Some(3));
}

#[test]
fn a_conversation_posts_into_two_topics_through_two_default_threads() {
    let store = store_with_topics(&["one", "two"]);
    let repository = store.repository();
    repository
        .insert_thread(&implicit_thread("d1", "one", "conversation-1"))
        .unwrap();
    repository
        .insert_thread(&implicit_thread("d2", "two", "conversation-1"))
        .unwrap();
    repository
        .insert_message(&plain("m1", "one", "d1"), NOW)
        .unwrap();
    repository
        .insert_message(&plain("m2", "two", "d2"), NOW)
        .unwrap();
    let snapshot = repository.grouping_snapshot().unwrap();
    assert_eq!(snapshot.topics.len(), 2);
    assert_eq!(snapshot.threads.len(), 2);
    assert_eq!(snapshot.default_links.len(), 2);
}

#[test]
fn the_snapshot_carries_everything_core_resolution_reads() {
    let store = store_with_topics(&["one"]);
    let repository = store.repository();
    repository
        .insert_topic(&general_topic("g", "claude-code"))
        .unwrap();
    repository
        .insert_thread(&implicit_thread("d", "g", "conversation-1"))
        .unwrap();
    repository.insert_thread(&thread("t", "one")).unwrap();
    let snapshot = repository.grouping_snapshot().unwrap();
    assert_eq!(snapshot.topics.len(), 2);
    assert_eq!(snapshot.threads.len(), 2);
    assert_eq!(snapshot.default_links.len(), 1);
    assert_eq!(snapshot.default_links[0].thread_id, thread_id("d"));
}

#[test]
fn updating_a_thread_saves_title_mute_and_status() {
    let store = store_with_topics(&["one"]);
    let repository = store.repository();
    repository.insert_thread(&thread("a", "one")).unwrap();
    let mut changed = thread("a", "one");
    changed.title = "Renamed".to_owned();
    changed.muted = true;
    changed.updated_at = later(2);
    repository.update_thread(&changed).unwrap();
    let stored = repository.thread(&thread_id("a")).unwrap();
    assert_eq!(stored.title, "Renamed");
    assert!(stored.muted);
    assert!(matches!(
        repository
            .update_thread(&thread("ghost", "one"))
            .unwrap_err(),
        StoreError::NotFound {
            entity: EntityKind::Thread,
            ..
        }
    ));
}

// --- takeover ---------------------------------------------------------------------

#[test]
fn the_takeover_moves_messages_and_deletes_the_implicit_thread() {
    let store = store_with_topics(&["one"]);
    let repository = store.repository();
    let mut implicit = implicit_thread("implicit", "one", "conversation-1");
    implicit.muted = true;
    repository.insert_thread(&implicit).unwrap();
    repository
        .touch_default_thread("claude-code", "conversation-1", &topic_id("one"), later(7))
        .unwrap();
    for name in ["a", "b", "c"] {
        repository
            .insert_message(&plain(name, "one", "implicit"), NOW)
            .unwrap();
    }
    repository
        .insert_message(
            &closed(plain("done", "one", "implicit"), LifecycleState::Answered),
            NOW,
        )
        .unwrap();

    let explicit = explicit_default("explicit", "one", "conversation-1");
    let moved = repository
        .insert_thread_with_takeover(
            &explicit,
            &Takeover {
                replaces: thread_id("implicit"),
                inherits_mute: true,
            },
            later(10),
        )
        .unwrap();
    assert_eq!(moved, 4);

    assert!(matches!(
        repository.thread(&thread_id("implicit")).unwrap_err(),
        StoreError::NotFound {
            entity: EntityKind::Thread,
            ..
        }
    ));
    for name in ["a", "b", "c", "done"] {
        let message = repository.message(&message_id(name)).unwrap();
        assert_eq!(message.thread_id, thread_id("explicit"));
        assert_eq!(message.topic_id, topic_id("one"));
    }
    let stored = repository.thread(&thread_id("explicit")).unwrap();
    assert!(stored.muted, "mute moves in");
    assert_eq!(stored.default_for, explicit.default_for);
    let link = repository
        .default_thread_link("claude-code", "conversation-1", &topic_id("one"))
        .unwrap()
        .unwrap();
    assert_eq!(link.thread_id, thread_id("explicit"));
    assert_eq!(
        link.last_used,
        later(7),
        "the conversation keeps its last-used time"
    );
    assert_eq!(repository.default_links().unwrap().len(), 1);
}

#[test]
fn a_takeover_without_inherited_mute_leaves_the_new_thread_unmuted() {
    let store = store_with_topics(&["one"]);
    let repository = store.repository();
    repository
        .insert_thread(&implicit_thread("implicit", "one", "conversation-1"))
        .unwrap();
    repository
        .insert_thread_with_takeover(
            &explicit_default("explicit", "one", "conversation-1"),
            &Takeover {
                replaces: thread_id("implicit"),
                inherits_mute: false,
            },
            NOW,
        )
        .unwrap();
    assert!(!repository.thread(&thread_id("explicit")).unwrap().muted);
}

#[test]
fn a_failed_takeover_changes_nothing() {
    let store = store_with_topics(&["one"]);
    let repository = store.repository();
    repository
        .insert_thread(&implicit_thread("implicit", "one", "conversation-1"))
        .unwrap();
    repository
        .insert_message(&plain("a", "one", "implicit"), NOW)
        .unwrap();
    repository.insert_thread(&thread("taken", "one")).unwrap();

    let mut clashing = explicit_default("explicit", "one", "conversation-1");
    clashing.key = Some("key:taken".to_owned());
    let error = repository
        .insert_thread_with_takeover(
            &clashing,
            &Takeover {
                replaces: thread_id("implicit"),
                inherits_mute: false,
            },
            NOW,
        )
        .unwrap_err();
    assert!(matches!(
        error,
        StoreError::Conflict(Conflict::ThreadKey { .. })
    ));

    assert_eq!(
        repository.message(&message_id("a")).unwrap().thread_id,
        thread_id("implicit")
    );
    assert!(repository.thread(&thread_id("implicit")).is_ok());
    assert!(matches!(
        repository.thread(&thread_id("explicit")).unwrap_err(),
        StoreError::NotFound { .. }
    ));
    let link = repository
        .default_thread_link("claude-code", "conversation-1", &topic_id("one"))
        .unwrap()
        .unwrap();
    assert_eq!(link.thread_id, thread_id("implicit"));
}

#[test]
fn a_takeover_of_a_missing_thread_is_not_found() {
    let store = store_with_topics(&["one"]);
    let error = store
        .repository()
        .insert_thread_with_takeover(
            &explicit_default("explicit", "one", "conversation-1"),
            &Takeover {
                replaces: thread_id("ghost"),
                inherits_mute: false,
            },
            NOW,
        )
        .unwrap_err();
    assert!(matches!(
        error,
        StoreError::NotFound {
            entity: EntityKind::Thread,
            ..
        }
    ));
}

#[test]
#[should_panic(expected = "only an implicit thread")]
fn only_an_implicit_thread_can_be_taken_over() {
    let store = store_with_topics(&["one"]);
    let repository = store.repository();
    let mut explicit_first = explicit_default("first", "one", "conversation-1");
    explicit_first.implicit = false;
    repository.insert_thread(&explicit_first).unwrap();
    let _ = repository.insert_thread_with_takeover(
        &explicit_default("second", "one", "conversation-1"),
        &Takeover {
            replaces: thread_id("first"),
            inherits_mute: false,
        },
        NOW,
    );
}

// --- moves -------------------------------------------------------------------------

#[test]
fn moving_a_thread_moves_its_messages_topic_id() {
    let store = store_with_topics(&["one", "two"]);
    let repository = store.repository();
    repository.insert_thread(&thread("a", "one")).unwrap();
    repository.insert_thread(&thread("stays", "one")).unwrap();
    repository
        .insert_message(&plain("open", "one", "a"), NOW)
        .unwrap();
    repository
        .insert_message(
            &closed(plain("old", "one", "a"), LifecycleState::Dismissed),
            NOW,
        )
        .unwrap();
    repository
        .insert_message(&plain("other", "one", "stays"), NOW)
        .unwrap();

    repository
        .move_thread(&thread_id("a"), &topic_id("two"), later(1))
        .unwrap();

    assert_eq!(
        repository.thread(&thread_id("a")).unwrap().topic_id,
        topic_id("two")
    );
    for name in ["open", "old"] {
        assert_eq!(
            repository.message(&message_id(name)).unwrap().topic_id,
            topic_id("two")
        );
    }
    assert_eq!(
        repository.message(&message_id("other")).unwrap().topic_id,
        topic_id("one")
    );
    let in_two = MessageFilter {
        topic_id: Some(topic_id("two")),
        ..MessageFilter::default()
    };
    assert_eq!(repository.list_messages(&in_two).unwrap().len(), 1);
}

#[test]
fn moving_a_thread_into_a_topic_with_the_same_key_is_a_conflict() {
    let store = store_with_topics(&["one", "two"]);
    let repository = store.repository();
    repository.insert_thread(&thread("a", "one")).unwrap();
    let mut same_key = thread("b", "two");
    same_key.key = thread("a", "one").key;
    repository.insert_thread(&same_key).unwrap();
    repository
        .insert_message(&plain("m", "one", "a"), NOW)
        .unwrap();

    let error = repository
        .move_thread(&thread_id("a"), &topic_id("two"), NOW)
        .unwrap_err();
    assert!(matches!(
        error,
        StoreError::Conflict(Conflict::ThreadKey { .. })
    ));
    assert_eq!(
        repository.thread(&thread_id("a")).unwrap().topic_id,
        topic_id("one")
    );
    assert_eq!(
        repository.message(&message_id("m")).unwrap().topic_id,
        topic_id("one")
    );
}

#[test]
fn moving_a_default_thread_moves_its_link_unless_the_target_has_one() {
    let store = store_with_topics(&["one", "two", "three"]);
    let repository = store.repository();
    repository
        .insert_thread(&implicit_thread("d1", "one", "conversation-1"))
        .unwrap();
    repository
        .insert_thread(&implicit_thread("d3", "three", "conversation-1"))
        .unwrap();

    repository
        .move_thread(&thread_id("d1"), &topic_id("two"), NOW)
        .unwrap();
    assert!(
        repository
            .default_thread_link("claude-code", "conversation-1", &topic_id("two"))
            .unwrap()
            .is_some()
    );
    assert!(
        repository
            .default_thread_link("claude-code", "conversation-1", &topic_id("one"))
            .unwrap()
            .is_none()
    );

    let error = repository
        .move_thread(&thread_id("d1"), &topic_id("three"), NOW)
        .unwrap_err();
    assert!(matches!(
        error,
        StoreError::Conflict(Conflict::DefaultThread { .. })
    ));
}

#[test]
fn moving_a_thread_to_its_own_topic_or_a_missing_one() {
    let store = store_with_topics(&["one"]);
    let repository = store.repository();
    repository.insert_thread(&thread("a", "one")).unwrap();
    repository
        .move_thread(&thread_id("a"), &topic_id("one"), later(5))
        .unwrap();
    assert_eq!(repository.thread(&thread_id("a")).unwrap().updated_at, NOW);
    assert!(matches!(
        repository
            .move_thread(&thread_id("a"), &topic_id("ghost"), NOW)
            .unwrap_err(),
        StoreError::NotFound {
            entity: EntityKind::Topic,
            ..
        }
    ));
    assert!(matches!(
        repository
            .move_thread(&thread_id("ghost"), &topic_id("one"), NOW)
            .unwrap_err(),
        StoreError::NotFound {
            entity: EntityKind::Thread,
            ..
        }
    ));
}

#[test]
fn moving_a_message_changes_its_thread_and_topic() {
    let store = store_with_topics(&["one", "two"]);
    let repository = store.repository();
    repository.insert_thread(&thread("a", "one")).unwrap();
    repository.insert_thread(&thread("b", "two")).unwrap();
    repository
        .insert_message(&plain("m", "one", "a"), NOW)
        .unwrap();

    repository
        .move_message_to_thread(&message_id("m"), &thread_id("b"), NOW)
        .unwrap();
    let moved = repository.message(&message_id("m")).unwrap();
    assert_eq!(
        (moved.thread_id, moved.topic_id),
        (thread_id("b"), topic_id("two"))
    );

    assert!(matches!(
        repository
            .move_message_to_thread(&message_id("ghost"), &thread_id("b"), NOW)
            .unwrap_err(),
        StoreError::NotFound {
            entity: EntityKind::Message,
            ..
        }
    ));
    assert!(matches!(
        repository
            .move_message_to_thread(&message_id("m"), &thread_id("ghost"), NOW)
            .unwrap_err(),
        StoreError::NotFound {
            entity: EntityKind::Thread,
            ..
        }
    ));
}

// --- mark read -----------------------------------------------------------------------

fn store_for_mark_read() -> Store {
    let store = store_with_topics(&["one", "two"]);
    let repository = store.repository();
    repository.insert_thread(&thread("a", "one")).unwrap();
    repository.insert_thread(&thread("b", "one")).unwrap();
    repository.insert_thread(&thread("c", "two")).unwrap();
    repository
        .insert_message(&plain("a1", "one", "a"), NOW)
        .unwrap();
    repository
        .insert_message(&plain("a2", "one", "a"), NOW)
        .unwrap();
    repository
        .insert_message(&plain("b1", "one", "b"), NOW)
        .unwrap();
    repository
        .insert_message(&plain("c1", "two", "c"), NOW)
        .unwrap();
    repository
        .insert_message(
            &closed(plain("a3", "one", "a"), LifecycleState::Answered),
            NOW,
        )
        .unwrap();
    store
}

fn state_of(store: &Store, name: &str) -> LifecycleState {
    store
        .repository()
        .message(&message_id(name))
        .unwrap()
        .lifecycle
        .state
}

#[test]
fn mark_read_per_topic_acknowledges_every_open_message_in_it() {
    let store = store_for_mark_read();
    let repository = store.repository();
    let acknowledged = repository
        .mark_topic_read(&topic_id("one"), later(1))
        .unwrap();
    assert_eq!(
        acknowledged,
        [message_id("a1"), message_id("a2"), message_id("b1")]
    );

    assert_eq!(state_of(&store, "a1"), LifecycleState::Acknowledged);
    assert_eq!(state_of(&store, "b1"), LifecycleState::Acknowledged);
    assert_eq!(
        state_of(&store, "c1"),
        LifecycleState::Open,
        "another topic"
    );
    assert_eq!(
        state_of(&store, "a3"),
        LifecycleState::Answered,
        "nothing is answered or reopened"
    );
    let topic = repository.topic(&topic_id("one")).unwrap();
    assert_eq!(topic.marked_read_at, Some(later(1)));
    assert_eq!(
        repository
            .mark_topic_read(&topic_id("one"), later(2))
            .unwrap(),
        []
    );
}

#[test]
fn mark_read_per_thread_leaves_its_siblings_alone() {
    let store = store_for_mark_read();
    let repository = store.repository();
    let acknowledged = repository
        .mark_thread_read(&thread_id("a"), later(1))
        .unwrap();
    assert_eq!(acknowledged, [message_id("a1"), message_id("a2")]);
    assert_eq!(state_of(&store, "b1"), LifecycleState::Open);
    assert_eq!(
        repository.thread(&thread_id("a")).unwrap().marked_read_at,
        Some(later(1))
    );
    assert_eq!(
        repository.thread(&thread_id("b")).unwrap().marked_read_at,
        None
    );
    assert_eq!(
        repository.topic(&topic_id("one")).unwrap().marked_read_at,
        None
    );
}

#[test]
fn mark_read_acknowledges_exactly_as_the_core_lifecycle_does() {
    let store = store_for_mark_read();
    let before = store.repository().message(&message_id("a1")).unwrap();
    store
        .repository()
        .mark_topic_read(&topic_id("one"), later(1))
        .unwrap();
    let after = store.repository().message(&message_id("a1")).unwrap();
    let expected = apply_event(&before.lifecycle, &LifecycleEvent::Acknowledge, later(1)).unwrap();
    assert_eq!(after.lifecycle, expected);
}

#[test]
fn mark_read_of_a_missing_topic_or_thread_is_not_found() {
    let store = store_for_mark_read();
    assert!(matches!(
        store
            .repository()
            .mark_topic_read(&topic_id("ghost"), NOW)
            .unwrap_err(),
        StoreError::NotFound {
            entity: EntityKind::Topic,
            ..
        }
    ));
    assert!(matches!(
        store
            .repository()
            .mark_thread_read(&thread_id("ghost"), NOW)
            .unwrap_err(),
        StoreError::NotFound {
            entity: EntityKind::Thread,
            ..
        }
    ));
}

// --- close and reopen -------------------------------------------------------------------

fn store_with_three_threads() -> Store {
    let store = store_with_topics(&["one"]);
    let repository = store.repository();
    for name in ["a", "b", "c"] {
        repository.insert_thread(&thread(name, "one")).unwrap();
    }
    repository
        .insert_message(&plain("m", "one", "a"), NOW)
        .unwrap();
    store
}

#[test]
fn closing_a_topic_closes_its_open_threads_and_keeps_its_messages() {
    let store = store_with_three_threads();
    let repository = store.repository();
    repository.close_thread(&thread_id("c"), later(1)).unwrap();

    let closed_threads = repository.close_topic(&topic_id("one"), later(2)).unwrap();
    assert_eq!(closed_threads, [thread_id("a"), thread_id("b")]);

    let topic = repository.topic(&topic_id("one")).unwrap();
    assert_eq!(
        (topic.status, topic.closed_at),
        (GroupStatus::Closed, Some(later(2)))
    );
    let a = repository.thread(&thread_id("a")).unwrap();
    assert_eq!(
        (a.status, a.closed_by_topic, a.closed_at),
        (GroupStatus::Closed, true, Some(later(2)))
    );
    let c = repository.thread(&thread_id("c")).unwrap();
    assert_eq!(
        (c.closed_by_topic, c.closed_at),
        (false, Some(later(1))),
        "closed on its own earlier"
    );
    assert_eq!(state_of_in(&repository, "m"), LifecycleState::Open);
    assert_eq!(
        repository.close_topic(&topic_id("one"), later(3)).unwrap(),
        []
    );
    assert_eq!(
        repository.topic(&topic_id("one")).unwrap().closed_at,
        Some(later(2))
    );
}

fn state_of_in(repository: &gentbox_store::Repository<'_>, name: &str) -> LifecycleState {
    repository
        .message(&message_id(name))
        .unwrap()
        .lifecycle
        .state
}

#[test]
fn reopening_a_topic_reopens_only_the_threads_its_closing_closed() {
    let store = store_with_three_threads();
    let repository = store.repository();
    repository.close_thread(&thread_id("c"), later(1)).unwrap();
    repository.close_topic(&topic_id("one"), later(2)).unwrap();

    let reopened = repository.reopen_topic(&topic_id("one"), later(3)).unwrap();
    assert_eq!(reopened, [thread_id("a"), thread_id("b")]);

    let topic = repository.topic(&topic_id("one")).unwrap();
    assert_eq!((topic.status, topic.closed_at), (GroupStatus::Open, None));
    for name in ["a", "b"] {
        let thread = repository.thread(&thread_id(name)).unwrap();
        assert_eq!(
            (thread.status, thread.closed_by_topic, thread.closed_at),
            (GroupStatus::Open, false, None)
        );
    }
    assert_eq!(
        repository.thread(&thread_id("c")).unwrap().status,
        GroupStatus::Closed
    );
    assert_eq!(
        repository.reopen_topic(&topic_id("one"), later(4)).unwrap(),
        []
    );
}

#[test]
fn a_message_to_a_closed_thread_reopens_it_and_its_closed_topic() {
    let store = store_with_three_threads();
    let repository = store.repository();
    repository.close_topic(&topic_id("one"), later(1)).unwrap();

    let reopened = repository.reopen_thread(&thread_id("a"), later(2)).unwrap();
    assert!(reopened.thread && reopened.topic);
    assert_eq!(
        repository.topic(&topic_id("one")).unwrap().status,
        GroupStatus::Open
    );
    assert_eq!(
        repository.thread(&thread_id("a")).unwrap().status,
        GroupStatus::Open
    );
    // Siblings the topic closed stay closed, and no longer wait for the topic.
    let b = repository.thread(&thread_id("b")).unwrap();
    assert_eq!((b.status, b.closed_by_topic), (GroupStatus::Closed, false));

    let nothing = repository.reopen_thread(&thread_id("a"), later(3)).unwrap();
    assert!(!nothing.thread && !nothing.topic);
}

#[test]
fn reopening_a_thread_closed_alone_leaves_its_open_topic_alone() {
    let store = store_with_three_threads();
    let repository = store.repository();
    assert!(repository.close_thread(&thread_id("b"), later(1)).unwrap());
    assert!(!repository.close_thread(&thread_id("b"), later(2)).unwrap());
    let reopened = repository.reopen_thread(&thread_id("b"), later(3)).unwrap();
    assert!(reopened.thread && !reopened.topic);
    assert_eq!(repository.thread(&thread_id("b")).unwrap().closed_at, None);
}

#[test]
fn closing_and_reopening_a_missing_topic_or_thread_is_not_found() {
    let store = store_with_three_threads();
    let repository = store.repository();
    assert!(matches!(
        repository.close_topic(&topic_id("ghost"), NOW).unwrap_err(),
        StoreError::NotFound { .. }
    ));
    assert!(matches!(
        repository
            .reopen_topic(&topic_id("ghost"), NOW)
            .unwrap_err(),
        StoreError::NotFound { .. }
    ));
    assert!(matches!(
        repository
            .close_thread(&thread_id("ghost"), NOW)
            .unwrap_err(),
        StoreError::NotFound { .. }
    ));
    assert!(matches!(
        repository
            .reopen_thread(&thread_id("ghost"), NOW)
            .unwrap_err(),
        StoreError::NotFound { .. }
    ));
}

// --- records ---------------------------------------------------------------------------

#[test]
fn records_carry_the_live_counts() {
    let store = store_with_three_threads();
    let repository = store.repository();
    repository
        .insert_message(&plain("m2", "one", "b"), NOW)
        .unwrap();
    repository
        .insert_message(
            &closed(plain("m3", "one", "b"), LifecycleState::Answered),
            NOW,
        )
        .unwrap();
    repository.close_thread(&thread_id("c"), NOW).unwrap();

    let record: TopicRecord = repository.topic_record(&topic_id("one")).unwrap();
    assert_eq!(
        (record.open_messages, record.open_threads),
        (Some(2), Some(2))
    );
    let thread: ThreadRecord = repository.thread_record(&thread_id("b")).unwrap();
    assert_eq!(
        (thread.open_messages, thread.conversations),
        (Some(1), Some(1))
    );

    let listed = repository
        .list_thread_records(Some(&topic_id("one")), Some(GroupStatus::Open))
        .unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(
        repository
            .list_topic_records(Some(GroupStatus::Closed))
            .unwrap(),
        []
    );
    assert_eq!(repository.list_topic_records(None).unwrap().len(), 1);
}

#[test]
fn a_moved_thread_is_no_longer_held_closed_by_its_old_topic() {
    let store = store_with_topics(&["a", "b"]);
    let repository = store.repository();
    repository.insert_thread(&thread("t", "a")).unwrap();
    repository.close_topic(&topic_id("a"), later(1)).unwrap();
    assert!(repository.thread(&thread_id("t")).unwrap().closed_by_topic);

    repository
        .move_thread(&thread_id("t"), &topic_id("b"), later(2))
        .unwrap();
    assert!(!repository.thread(&thread_id("t")).unwrap().closed_by_topic);

    // Closing and reopening the new topic must not reopen it, and nor does
    // reopening the old one.
    repository.close_topic(&topic_id("b"), later(3)).unwrap();
    assert_eq!(
        repository.reopen_topic(&topic_id("b"), later(4)).unwrap(),
        []
    );
    assert_eq!(
        repository.reopen_topic(&topic_id("a"), later(5)).unwrap(),
        []
    );
    assert_eq!(
        repository.thread(&thread_id("t")).unwrap().status,
        GroupStatus::Closed
    );
}

#[test]
fn a_closed_group_must_carry_its_closing_time_and_an_open_one_must_not() {
    let store = store_with_topics(&["one"]);
    let repository = store.repository();

    let mut closed_without_time = topic("bad-closed");
    closed_without_time.status = GroupStatus::Closed;
    assert!(matches!(
        repository.insert_topic(&closed_without_time).unwrap_err(),
        StoreError::Constraint(_)
    ));
    let mut open_with_time = topic("bad-open");
    open_with_time.closed_at = Some(later(1));
    assert!(matches!(
        repository.insert_topic(&open_with_time).unwrap_err(),
        StoreError::Constraint(_)
    ));
    let mut updated = topic("one");
    updated.status = GroupStatus::Closed;
    assert!(matches!(
        repository.update_topic(&updated).unwrap_err(),
        StoreError::Constraint(_)
    ));

    let mut bad_thread = thread("bad", "one");
    bad_thread.status = GroupStatus::Closed;
    assert!(matches!(
        repository.insert_thread(&bad_thread).unwrap_err(),
        StoreError::Constraint(_)
    ));
    repository.insert_thread(&thread("fine", "one")).unwrap();
    let mut reopened_wrongly = thread("fine", "one");
    reopened_wrongly.closed_at = Some(later(1));
    assert!(matches!(
        repository.update_thread(&reopened_wrongly).unwrap_err(),
        StoreError::Constraint(_)
    ));
}
