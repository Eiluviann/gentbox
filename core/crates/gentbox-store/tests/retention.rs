//! The retention sweep: which messages, threads, topics and conversations it
//! deletes and which it keeps.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use gentbox_core::ids::ConversationKey;
use gentbox_core::lifecycle::LifecycleState;
use gentbox_core::message::Conversation;
use gentbox_core::message::{ActionKind, Message};
use gentbox_core::topic::GroupStatus;
use gentbox_store::{
    DEFAULT_RETENTION, DeliveryEvent, DeliveryRecord, EntityKind, MessageFilter, RetentionPolicy,
    Store, StoreError, SweepReport,
};
use support::{
    NOW, closed, general_topic, implicit_thread, memory_store, plain, seed_group, thread,
    thread_id, topic, topic_id,
};
use time::Duration;

fn days(count: i64) -> Duration {
    Duration::days(count)
}

/// Inserts a live message, then closes it `age_days` before `NOW`.
fn insert_closed(store: &Store, name: &str, state: LifecycleState, age_days: i64) {
    let repository = store.repository();
    let live = plain(name, "one", "one");
    repository.insert_message(&live, NOW - days(100)).unwrap();
    repository
        .update_message(&closed(live, state), NOW - days(age_days))
        .unwrap();
}

fn survivors(store: &Store) -> Vec<String> {
    let everything = MessageFilter {
        states: Some(LifecycleState::ALL.to_vec()),
        ..MessageFilter::default()
    };
    let mut names: Vec<String> = store
        .repository()
        .list_messages(&everything)
        .unwrap()
        .iter()
        .map(|message| {
            message
                .id
                .as_str()
                .trim_start_matches("message-")
                .to_owned()
        })
        .collect();
    names.sort();
    names
}

fn store_with_group() -> Store {
    let store = memory_store();
    seed_group(&store.repository(), "one");
    store
}

#[test]
fn it_deletes_only_answered_dismissed_rescinded_and_expired_messages_older_than_the_cutoff() {
    let store = store_with_group();
    for (name, state) in [
        ("answered", LifecycleState::Answered),
        ("dismissed", LifecycleState::Dismissed),
        ("rescinded", LifecycleState::Rescinded),
        ("expired", LifecycleState::Expired),
    ] {
        insert_closed(&store, &format!("{name}-old"), state, 31);
        insert_closed(&store, &format!("{name}-new"), state, 29);
    }
    // Open and acknowledged messages are never deleted, however old.
    let repository = store.repository();
    repository
        .insert_message(&plain("open-ancient", "one", "one"), NOW - days(400))
        .unwrap();
    let mut acknowledged = plain("acknowledged-ancient", "one", "one");
    acknowledged.lifecycle.state = LifecycleState::Acknowledged;
    acknowledged.received_at = NOW - days(400);
    repository
        .insert_message(&acknowledged, NOW - days(400))
        .unwrap();

    let report = repository.sweep(NOW, &RetentionPolicy::default()).unwrap();
    assert_eq!(report.messages, 4);
    assert_eq!(
        survivors(&store),
        [
            "acknowledged-ancient",
            "answered-new",
            "dismissed-new",
            "expired-new",
            "open-ancient",
            "rescinded-new",
        ]
    );
}

#[test]
fn the_cutoff_is_strict_and_counts_from_when_the_message_closed() {
    let store = store_with_group();
    insert_closed(&store, "exactly-30-days", LifecycleState::Answered, 30);
    insert_closed(&store, "closed-now", LifecycleState::Expired, 0);
    let repository = store.repository();
    let mut barely = plain("barely-past", "one", "one");
    barely.received_at = NOW - days(100);
    repository.insert_message(&barely, NOW - days(100)).unwrap();
    repository
        .update_message(
            &closed(barely, LifecycleState::Answered),
            NOW - days(30) - Duration::nanoseconds(1),
        )
        .unwrap();

    repository.sweep(NOW, &RetentionPolicy::default()).unwrap();
    // "closed-now" was received 100 days ago but closed just now: kept.
    assert_eq!(survivors(&store), ["closed-now", "exactly-30-days"]);
}

#[test]
fn the_retention_period_is_configurable() {
    let store = store_with_group();
    insert_closed(&store, "eight-days", LifecycleState::Answered, 8);
    insert_closed(&store, "two-days", LifecycleState::Answered, 2);
    let policy = RetentionPolicy {
        message_retention: days(7),
        group_retention: days(7),
    };
    let report = store.repository().sweep(NOW, &policy).unwrap();
    assert_eq!(report.messages, 1);
    assert_eq!(survivors(&store), ["two-days"]);
    assert_eq!(DEFAULT_RETENTION, days(30));
}

#[test]
fn deleting_a_message_deletes_its_actions_answer_and_delivery_log() {
    let store = store_with_group();
    let repository = store.repository();
    let mut message = plain("gone", "one", "one");
    message.actions = vec![gentbox_core::message::Action {
        id: "ok".to_owned(),
        label: "OK".to_owned(),
        kind: ActionKind::Approve,
        value: None,
        placeholder: None,
        destructive: false,
    }];
    repository
        .insert_message(&message, NOW - days(100))
        .unwrap();
    repository
        .append_delivery(&DeliveryRecord {
            message_id: message.id.clone(),
            recorded_at: NOW - days(100),
            event: DeliveryEvent::Delivered,
            surface: Some("popup".to_owned()),
            detail: None,
        })
        .unwrap();
    let mut answered = closed(message, LifecycleState::Answered);
    answered.lifecycle.answer = Some(gentbox_core::lifecycle::RecordedAnswer {
        action_id: "ok".to_owned(),
        action_kind: ActionKind::Approve,
        value: None,
        answered_via: None,
        answered_at: NOW - days(40),
    });
    repository
        .update_message(&answered, NOW - days(40))
        .unwrap();

    assert_eq!(
        repository
            .sweep(NOW, &RetentionPolicy::default())
            .unwrap()
            .messages,
        1
    );
    // A new message with the same id and actions inserts cleanly: no row of
    // the old one is left behind to clash with it.
    let mut again = plain("gone", "one", "one");
    again.actions = answered.actions;
    repository.insert_message(&again, NOW).unwrap();
    assert_eq!(repository.deliveries(&again.id).unwrap(), []);
    assert_eq!(
        repository.message(&again.id).unwrap().lifecycle.answer,
        None
    );
}

#[test]
fn deleting_a_message_clears_the_supersede_links_that_pointed_at_it() {
    let store = store_with_group();
    let repository = store.repository();
    let old = plain("old", "one", "one");
    let new = plain("new", "one", "one");
    repository.insert_message(&old, NOW - days(100)).unwrap();
    repository.insert_message(&new, NOW - days(100)).unwrap();
    repository.record_supersession(&old.id, &new.id).unwrap();
    repository
        .update_message(
            &closed(new.clone(), LifecycleState::Answered),
            NOW - days(40),
        )
        .unwrap();

    repository.sweep(NOW, &RetentionPolicy::default()).unwrap();
    assert_eq!(survivors(&store), ["old"]);
    assert_eq!(repository.superseded_by(&old.id).unwrap(), None);
}

// --- groups ---------------------------------------------------------------------

fn group_names(store: &Store) -> (Vec<String>, Vec<String>) {
    let repository = store.repository();
    let topics = repository
        .list_topics(None)
        .unwrap()
        .into_iter()
        .map(|topic| topic.id.to_string())
        .collect();
    let threads = repository
        .list_threads(None, None)
        .unwrap()
        .into_iter()
        .map(|thread| thread.id.to_string())
        .collect();
    (topics, threads)
}

#[test]
fn a_topic_or_thread_with_messages_is_never_deleted() {
    let store = store_with_group();
    let repository = store.repository();
    repository
        .insert_message(&plain("m", "one", "one"), NOW - days(200))
        .unwrap();
    repository
        .close_topic(&topic_id("one"), NOW - days(100))
        .unwrap();

    let report = repository.sweep(NOW, &RetentionPolicy::default()).unwrap();
    assert_eq!((report.threads, report.topics), (0, 0));
    assert_eq!(
        group_names(&store),
        (vec!["topic-one".to_owned()], vec!["thread-one".to_owned()])
    );
}

#[test]
fn a_group_whose_messages_were_all_deleted_goes_with_them() {
    let store = store_with_group();
    insert_closed(&store, "old", LifecycleState::Answered, 40);
    let repository = store.repository();
    repository
        .close_topic(&topic_id("one"), NOW - days(35))
        .unwrap();

    let report = repository.sweep(NOW, &RetentionPolicy::default()).unwrap();
    assert_eq!(
        report,
        SweepReport {
            messages: 1,
            threads: 1,
            topics: 1,
            conversations: 1
        }
    );
    assert_eq!(group_names(&store), (vec![], vec![]));
}

#[test]
fn closed_empty_groups_are_deleted_only_after_the_retention_period() {
    let store = memory_store();
    let repository = store.repository();
    for name in ["old", "recent"] {
        repository.insert_topic(&topic(name)).unwrap();
        repository.insert_thread(&thread(name, name)).unwrap();
    }
    repository
        .close_topic(&topic_id("old"), NOW - days(31))
        .unwrap();
    repository
        .close_topic(&topic_id("recent"), NOW - days(29))
        .unwrap();

    let report = repository.sweep(NOW, &RetentionPolicy::default()).unwrap();
    assert_eq!((report.threads, report.topics), (1, 1));
    assert_eq!(
        group_names(&store),
        (
            vec!["topic-recent".to_owned()],
            vec!["thread-recent".to_owned()]
        )
    );
}

#[test]
fn a_closed_thread_goes_but_its_open_topic_stays() {
    let store = memory_store();
    let repository = store.repository();
    repository.insert_topic(&topic("one")).unwrap();
    repository.insert_thread(&thread("closed", "one")).unwrap();
    repository.insert_thread(&thread("open", "one")).unwrap();
    repository
        .close_thread(&thread_id("closed"), NOW - days(31))
        .unwrap();

    let report = repository.sweep(NOW, &RetentionPolicy::default()).unwrap();
    assert_eq!((report.threads, report.topics), (1, 0));
    assert_eq!(group_names(&store).1, ["thread-open"]);
}

#[test]
fn an_unmuted_implicit_empty_thread_is_deleted_but_the_general_topic_stays() {
    let store = memory_store();
    let repository = store.repository();
    let mut general = general_topic("general", "claude-code");
    general.updated_at = NOW - days(45);
    general.created_at = NOW - days(45);
    repository.insert_topic(&general).unwrap();
    let mut implicit = implicit_thread("implicit", "general", "conversation-1");
    implicit.updated_at = NOW - days(45);
    implicit.created_at = NOW - days(45);
    repository.insert_thread(&implicit).unwrap();

    let report = repository.sweep(NOW, &RetentionPolicy::default()).unwrap();
    assert_eq!((report.threads, report.topics), (1, 0));
    assert_eq!(
        group_names(&store),
        (vec!["topic-general".to_owned()], vec![])
    );
    assert!(repository.default_links().unwrap().is_empty());
}

#[test]
fn a_general_topic_survives_empty_past_the_cutoff_even_when_closed() {
    let store = memory_store();
    let repository = store.repository();
    let mut general = general_topic("general", "claude-code");
    general.updated_at = NOW - days(400);
    general.created_at = NOW - days(400);
    repository.insert_topic(&general).unwrap();
    repository
        .close_topic(&topic_id("general"), NOW - days(300))
        .unwrap();

    assert_eq!(
        repository.sweep(NOW, &RetentionPolicy::default()).unwrap(),
        SweepReport::default()
    );
    assert!(repository.topic(&topic_id("general")).is_ok());
}

#[test]
fn a_muted_empty_topic_and_thread_survive_past_the_cutoff() {
    let store = memory_store();
    let repository = store.repository();
    for name in ["muted-closed", "muted-implicit-thread"] {
        let mut muted_topic = topic(name);
        muted_topic.muted = true;
        repository.insert_topic(&muted_topic).unwrap();
    }
    repository
        .close_topic(&topic_id("muted-closed"), NOW - days(100))
        .unwrap();

    let mut muted_closed_thread = thread("muted-closed-thread", "muted-implicit-thread");
    muted_closed_thread.muted = true;
    repository.insert_thread(&muted_closed_thread).unwrap();
    repository
        .close_thread(&thread_id("muted-closed-thread"), NOW - days(100))
        .unwrap();
    let mut muted_implicit = implicit_thread("muted-implicit", "muted-implicit-thread", "c1");
    muted_implicit.muted = true;
    muted_implicit.updated_at = NOW - days(100);
    repository.insert_thread(&muted_implicit).unwrap();

    assert_eq!(
        repository.sweep(NOW, &RetentionPolicy::default()).unwrap(),
        SweepReport::default()
    );
    assert!(repository.topic(&topic_id("muted-closed")).unwrap().muted);
    assert!(
        repository
            .thread(&thread_id("muted-implicit"))
            .unwrap()
            .muted
    );
    assert!(
        repository
            .thread(&thread_id("muted-closed-thread"))
            .unwrap()
            .muted
    );
}

#[test]
fn unmuting_makes_a_group_deletable_again() {
    let store = memory_store();
    let repository = store.repository();
    let mut muted = topic("one");
    muted.muted = true;
    repository.insert_topic(&muted).unwrap();
    repository
        .close_topic(&topic_id("one"), NOW - days(100))
        .unwrap();
    assert_eq!(
        repository
            .sweep(NOW, &RetentionPolicy::default())
            .unwrap()
            .topics,
        0
    );
    let mut unmuted = repository.topic(&topic_id("one")).unwrap();
    unmuted.muted = false;
    repository.update_topic(&unmuted).unwrap();
    assert_eq!(
        repository
            .sweep(NOW, &RetentionPolicy::default())
            .unwrap()
            .topics,
        1
    );
}

#[test]
fn unmuted_empty_closed_and_implicit_groups_are_still_deleted() {
    let store = memory_store();
    let repository = store.repository();
    repository.insert_topic(&topic("closed")).unwrap();
    repository
        .close_topic(&topic_id("closed"), NOW - days(60))
        .unwrap();
    repository.insert_topic(&topic("host")).unwrap();
    let mut implicit = implicit_thread("implicit", "host", "conversation-1");
    implicit.updated_at = NOW - days(60);
    repository.insert_thread(&implicit).unwrap();
    repository
        .insert_thread(&thread("closed-thread", "host"))
        .unwrap();
    repository
        .close_thread(&thread_id("closed-thread"), NOW - days(60))
        .unwrap();

    let report = repository.sweep(NOW, &RetentionPolicy::default()).unwrap();
    assert_eq!((report.threads, report.topics), (2, 1));
    assert_eq!(group_names(&store), (vec!["topic-host".to_owned()], vec![]));
}

#[test]
fn a_recently_used_implicit_group_is_kept() {
    let store = memory_store();
    let repository = store.repository();
    let mut general = general_topic("general", "claude-code");
    general.updated_at = NOW - days(1);
    repository.insert_topic(&general).unwrap();
    let report = repository.sweep(NOW, &RetentionPolicy::default()).unwrap();
    assert_eq!(report.topics, 0);
}

#[test]
fn open_explicit_empty_groups_are_kept_however_old() {
    let store = memory_store();
    let repository = store.repository();
    let mut old_topic = topic("one");
    old_topic.created_at = NOW - days(500);
    old_topic.updated_at = NOW - days(500);
    repository.insert_topic(&old_topic).unwrap();
    let mut old_thread = thread("one", "one");
    old_thread.created_at = NOW - days(500);
    old_thread.updated_at = NOW - days(500);
    repository.insert_thread(&old_thread).unwrap();

    assert_eq!(
        repository.sweep(NOW, &RetentionPolicy::default()).unwrap(),
        SweepReport::default()
    );
}

#[test]
fn a_topic_survives_while_any_of_its_threads_does() {
    let store = memory_store();
    let repository = store.repository();
    repository.insert_topic(&topic("one")).unwrap();
    repository
        .insert_thread(&thread("with-message", "one"))
        .unwrap();
    repository.insert_thread(&thread("empty", "one")).unwrap();
    repository
        .insert_message(&plain("m", "one", "with-message"), NOW - days(60))
        .unwrap();
    repository
        .close_topic(&topic_id("one"), NOW - days(50))
        .unwrap();

    let report = repository.sweep(NOW, &RetentionPolicy::default()).unwrap();
    assert_eq!((report.threads, report.topics), (1, 0));
    assert_eq!(
        repository.topic(&topic_id("one")).unwrap().status,
        GroupStatus::Closed
    );
    assert!(repository.thread(&thread_id("with-message")).is_ok());
}

// --- conversations -----------------------------------------------------------------

#[test]
fn conversations_nothing_refers_to_are_deleted() {
    let store = store_with_group();
    let repository = store.repository();
    let stale = Conversation {
        id: "stale".to_owned(),
        title: None,
        cwd: None,
        focus: None,
    };
    let fresh = Conversation {
        id: "fresh".to_owned(),
        ..stale.clone()
    };
    repository
        .upsert_conversation(
            &ConversationKey::new("claude-code", "stale"),
            &stale,
            NOW - days(60),
        )
        .unwrap();
    repository
        .upsert_conversation(
            &ConversationKey::new("claude-code", "fresh"),
            &fresh,
            NOW - days(1),
        )
        .unwrap();
    // Referred to by a live message: kept.
    let mut kept = plain("m", "one", "one");
    kept.conversation.id = "busy".to_owned();
    repository.insert_message(&kept, NOW - days(90)).unwrap();

    let report = repository.sweep(NOW, &RetentionPolicy::default()).unwrap();
    assert_eq!(report.conversations, 1);
    assert!(matches!(
        repository
            .conversation(&ConversationKey::new("claude-code", "stale"))
            .unwrap_err(),
        StoreError::NotFound {
            entity: EntityKind::Conversation,
            ..
        }
    ));
    assert!(
        repository
            .conversation(&ConversationKey::new("claude-code", "fresh"))
            .is_ok()
    );
    assert!(repository.conversation(&kept.conversation_key()).is_ok());
}

// --- the sweep itself ----------------------------------------------------------------------

#[test]
fn a_second_sweep_finds_nothing_more() {
    let store = store_with_group();
    insert_closed(&store, "old", LifecycleState::Answered, 40);
    let repository = store.repository();
    assert_ne!(
        repository.sweep(NOW, &RetentionPolicy::default()).unwrap(),
        SweepReport::default()
    );
    assert_eq!(
        repository.sweep(NOW, &RetentionPolicy::default()).unwrap(),
        SweepReport::default()
    );
}

#[test]
fn sweeping_an_empty_store_deletes_nothing() {
    let store = memory_store();
    assert_eq!(
        store
            .repository()
            .sweep(NOW, &RetentionPolicy::default())
            .unwrap(),
        SweepReport::default()
    );
}

#[test]
fn the_sweep_works_on_messages_of_every_combination() {
    let store = store_with_group();
    let repository = store.repository();
    let mut messages: Vec<Message> = Vec::new();
    for (index, key) in gentbox_core::policy::PolicyKey::all_valid()
        .iter()
        .enumerate()
    {
        messages.push(support::classified(
            &format!("c{index}"),
            "one",
            "one",
            key.message_type,
            key.urgent,
            key.blocking,
        ));
    }
    for message in &messages {
        repository.insert_message(message, NOW - days(100)).unwrap();
        repository
            .update_message(
                &closed(message.clone(), LifecycleState::Expired),
                NOW - days(31),
            )
            .unwrap();
    }
    assert_eq!(
        repository
            .sweep(NOW, &RetentionPolicy::default())
            .unwrap()
            .messages,
        15
    );
}
