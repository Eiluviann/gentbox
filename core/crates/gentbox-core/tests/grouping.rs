//! Topics and threads: resolution, creates, takeover, templates and mute
//! (ADRs 0008, 0009).
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;

use gentbox_core::ids::{ThreadId, TopicId};
use gentbox_core::message::Conversation;
use gentbox_core::policy::{MacSurface, Policy, decide};
use gentbox_core::resolve::{
    DefaultThreadLink, GroupingError, GroupingRequest, GroupingSnapshot, NewThread, NewTopic,
    Resolution, Takeover, ThreadCreateAction, ThreadCreateContext, ThreadTarget,
    TopicCreateOutcome, TopicTarget, create_thread, create_topic, resolve_grouping,
};
use gentbox_core::template::{GroupSpec, GroupTemplate, TemplateVars, is_valid_group_key};
use gentbox_core::topic::{
    CreatedBy, DefaultFor, GroupStatus, Thread, ThreadCreate, ThreadRef, Topic, TopicCreate,
    TopicRef, is_effectively_muted,
};
use serde_json::json;
use support::{NOW, awaiting, calm_context, valid_combinations};
use time::Duration;

fn topic(id: &str, key: Option<&str>, title: &str) -> Topic {
    Topic {
        id: TopicId::new(id),
        key: key.map(str::to_owned),
        title: title.to_owned(),
        summary: None,
        kind: key.map(|_| "project".to_owned()),
        style: None,
        implicit: false,
        status: GroupStatus::Open,
        muted: false,
        marked_read_at: None,
        created_by: CreatedBy {
            agent: "claude-code".to_owned(),
            conversation_id: None,
        },
        created_at: NOW,
        updated_at: NOW,
        closed_at: None,
    }
}

fn general(id: &str, agent: &str) -> Topic {
    Topic {
        implicit: true,
        kind: Some("general".to_owned()),
        created_by: CreatedBy {
            agent: agent.to_owned(),
            conversation_id: None,
        },
        ..topic(id, None, "General")
    }
}

fn thread(id: &str, topic_id: &str, key: Option<&str>) -> Thread {
    Thread {
        id: ThreadId::new(id),
        topic_id: TopicId::new(topic_id),
        key: key.map(str::to_owned),
        title: key.unwrap_or("Conversation").to_owned(),
        summary: None,
        kind: None,
        implicit: key.is_none(),
        status: GroupStatus::Open,
        closed_by_topic: false,
        muted: false,
        marked_read_at: None,
        default_for: None,
        created_by: CreatedBy {
            agent: "claude-code".to_owned(),
            conversation_id: None,
        },
        created_at: NOW,
        updated_at: NOW,
        closed_at: None,
    }
}

fn link(conversation: &str, topic_id: &str, thread_id: &str, minutes: i64) -> DefaultThreadLink {
    DefaultThreadLink {
        agent: "claude-code".to_owned(),
        conversation_id: conversation.to_owned(),
        topic_id: TopicId::new(topic_id),
        thread_id: ThreadId::new(thread_id),
        last_used: NOW + Duration::minutes(minutes),
    }
}

fn request<'a>(conversation: &'a str) -> GroupingRequest<'a> {
    GroupingRequest {
        agent: "claude-code",
        conversation_id: conversation,
        topic: None,
        thread: None,
        rule_topic: None,
        rule_thread: None,
        conversation_title: Some("Fix flaky ingest test"),
        project: Some("gentbox"),
    }
}

fn topic_ref(value: serde_json::Value) -> TopicRef {
    serde_json::from_value(value).unwrap()
}

fn thread_ref(value: serde_json::Value) -> ThreadRef {
    serde_json::from_value(value).unwrap()
}

fn spec(key: &str, title: &str) -> GroupSpec {
    GroupSpec {
        key: key.to_owned(),
        title: title.to_owned(),
        kind: Some("project".to_owned()),
    }
}

fn existing_topic(resolution: &Resolution) -> &str {
    match &resolution.topic {
        TopicTarget::Existing(id) => id.as_str(),
        other => panic!("expected an existing topic, got {other:?}"),
    }
}

fn existing_thread(resolution: &Resolution) -> &str {
    match &resolution.thread {
        ThreadTarget::Existing(id) => id.as_str(),
        other => panic!("expected an existing thread, got {other:?}"),
    }
}

fn new_thread(resolution: &Resolution) -> &NewThread {
    match &resolution.thread {
        ThreadTarget::Create(thread) => thread,
        other => panic!("expected a new thread, got {other:?}"),
    }
}

fn new_topic(resolution: &Resolution) -> &NewTopic {
    match &resolution.topic {
        TopicTarget::Create(topic) => topic,
        other => panic!("expected a new topic, got {other:?}"),
    }
}

// --- thread.id fixes both ---

#[test]
fn a_thread_id_fixes_both_the_thread_and_its_topic() {
    let snapshot = GroupingSnapshot {
        topics: vec![topic("T1", Some("project:gentbox"), "gentbox")],
        threads: vec![thread("H1", "T1", Some("phase-3"))],
        default_links: vec![],
    };
    let thread = thread_ref(json!({ "id": "H1" }));
    let mut request = request("c1");
    request.thread = Some(&thread);
    let resolution = resolve_grouping(&snapshot, &request).unwrap();
    assert_eq!(existing_topic(&resolution), "T1");
    assert_eq!(existing_thread(&resolution), "H1");
}

#[test]
fn a_thread_id_with_a_matching_topic_is_fine_and_a_conflicting_topic_is_an_error() {
    let snapshot = GroupingSnapshot {
        topics: vec![
            topic("T1", Some("project:gentbox"), "gentbox"),
            topic("T2", Some("project:web"), "web"),
        ],
        threads: vec![thread("H1", "T1", Some("phase-3"))],
        default_links: vec![],
    };
    let thread = thread_ref(json!({ "id": "H1" }));
    for (label, named, conflicts) in [
        ("same id", json!({ "id": "T1" }), false),
        ("same key", json!({ "key": "project:gentbox" }), false),
        ("other id", json!({ "id": "T2" }), true),
        ("other key", json!({ "key": "project:web" }), true),
        ("unknown key", json!({ "key": "project:new" }), true),
    ] {
        let named = topic_ref(named);
        let mut request = request("c1");
        request.thread = Some(&thread);
        request.topic = Some(&named);
        let result = resolve_grouping(&snapshot, &request);
        assert_eq!(
            result == Err(GroupingError::TopicConflict),
            conflicts,
            "{label}"
        );
    }
}

#[test]
fn unknown_ids_are_errors() {
    let snapshot = GroupingSnapshot::default();
    let thread = thread_ref(json!({ "id": "NOPE" }));
    let mut request_for_thread = request("c1");
    request_for_thread.thread = Some(&thread);
    assert_eq!(
        resolve_grouping(&snapshot, &request_for_thread),
        Err(GroupingError::UnknownThread(ThreadId::new("NOPE")))
    );
    let topic = topic_ref(json!({ "id": "NOPE" }));
    let mut request_for_topic = request("c1");
    request_for_topic.topic = Some(&topic);
    assert_eq!(
        resolve_grouping(&snapshot, &request_for_topic),
        Err(GroupingError::UnknownTopic(TopicId::new("NOPE")))
    );
}

// --- topic resolution ---

#[test]
fn a_named_topic_by_id_or_existing_key_is_used_unchanged() {
    let snapshot = GroupingSnapshot {
        topics: vec![topic("T1", Some("project:gentbox"), "gentbox")],
        ..GroupingSnapshot::default()
    };
    for named in [
        json!({ "id": "T1" }),
        json!({ "key": "project:gentbox", "title": "Renamed" }),
    ] {
        let named = topic_ref(named);
        let mut request = request("c1");
        request.topic = Some(&named);
        assert_eq!(
            existing_topic(&resolve_grouping(&snapshot, &request).unwrap()),
            "T1"
        );
    }
}

#[test]
fn a_named_new_key_creates_the_topic_titled_by_title_or_key() {
    let snapshot = GroupingSnapshot::default();
    let named = topic_ref(json!({ "key": "release-1.4", "kind": "other" }));
    let mut request = request("c1");
    request.topic = Some(&named);
    let resolution = resolve_grouping(&snapshot, &request).unwrap();
    let created = new_topic(&resolution);
    assert_eq!(created.key.as_deref(), Some("release-1.4"));
    assert_eq!(created.title, "Release 1.4");
    assert_eq!(created.kind.as_deref(), Some("other"));
    assert!(!created.implicit);

    let titled = topic_ref(json!({ "key": "release-1.4", "title": "Release 1.4 train" }));
    request.topic = Some(&titled);
    assert_eq!(
        new_topic(&resolve_grouping(&snapshot, &request).unwrap()).title,
        "Release 1.4 train"
    );
}

#[test]
fn an_inline_title_that_looks_like_a_command_becomes_the_key_in_words() {
    let named = topic_ref(json!({ "key": "phase-3-ingest", "title": "cargo test -p ingest" }));
    let mut request = request("c1");
    request.topic = Some(&named);
    let resolution = resolve_grouping(&GroupingSnapshot::default(), &request).unwrap();
    assert_eq!(new_topic(&resolution).title, "Phase 3 ingest");
}

#[test]
fn topic_precedence_is_named_then_rule_then_latest_default_thread_then_general() {
    let snapshot = GroupingSnapshot {
        topics: vec![
            topic("T-named", Some("named"), "named"),
            topic("T-rule", Some("project:gentbox"), "gentbox"),
            topic("T-default", Some("older"), "older"),
            general("T-general", "claude-code"),
        ],
        threads: vec![thread("H-default", "T-default", None)],
        default_links: vec![link("c1", "T-default", "H-default", 0)],
    };
    let rule = spec("project:gentbox", "gentbox");
    let named = topic_ref(json!({ "key": "named" }));

    let mut req = request("c1");
    req.topic = Some(&named);
    req.rule_topic = Some(&rule);
    assert_eq!(
        existing_topic(&resolve_grouping(&snapshot, &req).unwrap()),
        "T-named"
    );

    req.topic = None;
    assert_eq!(
        existing_topic(&resolve_grouping(&snapshot, &req).unwrap()),
        "T-rule"
    );

    req.rule_topic = None;
    assert_eq!(
        existing_topic(&resolve_grouping(&snapshot, &req).unwrap()),
        "T-default"
    );

    let other_conversation = request("c2");
    assert_eq!(
        existing_topic(&resolve_grouping(&snapshot, &other_conversation).unwrap()),
        "T-general"
    );
}

#[test]
fn a_rule_assigned_topic_is_created_on_first_use() {
    let rule = spec("project:gentbox", "gentbox");
    let mut req = request("c1");
    req.rule_topic = Some(&rule);
    let resolution = resolve_grouping(&GroupingSnapshot::default(), &req).unwrap();
    let created = new_topic(&resolution);
    assert_eq!(created.key.as_deref(), Some("project:gentbox"));
    assert_eq!(created.kind.as_deref(), Some("project"));
}

#[test]
fn the_latest_default_thread_decides_the_topic() {
    let snapshot = GroupingSnapshot {
        topics: vec![
            topic("T-old", Some("old"), "old"),
            topic("T-new", Some("new"), "new"),
        ],
        threads: vec![
            thread("H-old", "T-old", None),
            thread("H-new", "T-new", None),
        ],
        default_links: vec![
            link("c1", "T-old", "H-old", 0),
            link("c1", "T-new", "H-new", 5),
        ],
    };
    assert_eq!(
        existing_topic(&resolve_grouping(&snapshot, &request("c1")).unwrap()),
        "T-new"
    );
}

#[test]
fn implicit_general_is_per_agent_and_created_on_first_use() {
    let first = resolve_grouping(&GroupingSnapshot::default(), &request("c1")).unwrap();
    let created = new_topic(&first);
    assert!(created.implicit);
    assert_eq!(created.key, None);
    assert_eq!(created.title, "General");
    assert_eq!(created.kind.as_deref(), Some("general"));

    let snapshot = GroupingSnapshot {
        topics: vec![
            general("G-claude", "claude-code"),
            general("G-codex", "codex"),
        ],
        ..GroupingSnapshot::default()
    };
    assert_eq!(
        existing_topic(&resolve_grouping(&snapshot, &request("c1")).unwrap()),
        "G-claude"
    );
    let mut codex = request("c1");
    codex.agent = "codex";
    assert_eq!(
        existing_topic(&resolve_grouping(&snapshot, &codex).unwrap()),
        "G-codex"
    );
    let mut other = request("c1");
    other.agent = "other-agent";
    assert!(new_topic(&resolve_grouping(&snapshot, &other).unwrap()).implicit);
}

// --- thread resolution ---

#[test]
fn a_named_thread_key_is_created_on_first_use_and_found_after() {
    let snapshot = GroupingSnapshot {
        topics: vec![topic("T1", Some("project:gentbox"), "gentbox")],
        threads: vec![thread("H1", "T1", Some("phase-3-ingest"))],
        default_links: vec![],
    };
    let topic = topic_ref(json!({ "key": "project:gentbox" }));
    let known = thread_ref(json!({ "key": "phase-3-ingest" }));
    let mut req = request("c1");
    req.topic = Some(&topic);
    req.thread = Some(&known);
    assert_eq!(
        existing_thread(&resolve_grouping(&snapshot, &req).unwrap()),
        "H1"
    );

    let fresh = thread_ref(json!({ "key": "phase-4", "title": "Phase 4", "kind": "phase" }));
    req.thread = Some(&fresh);
    let resolution = resolve_grouping(&snapshot, &req).unwrap();
    let created = new_thread(&resolution);
    assert_eq!(created.key.as_deref(), Some("phase-4"));
    assert_eq!(created.title, "Phase 4");
    assert_eq!(created.kind.as_deref(), Some("phase"));
    assert!(!created.implicit);
    assert_eq!(
        created.default_for, None,
        "the inline form never takes over"
    );
    assert!(!resolution.uses_default_thread);
}

#[test]
fn the_same_thread_key_in_different_topics_is_a_different_thread() {
    let snapshot = GroupingSnapshot {
        topics: vec![topic("T1", Some("a"), "a"), topic("T2", Some("b"), "b")],
        threads: vec![thread("H1", "T1", Some("v2"))],
        default_links: vec![],
    };
    let in_b = topic_ref(json!({ "key": "b" }));
    let key = thread_ref(json!({ "key": "v2" }));
    let mut req = request("c1");
    req.topic = Some(&in_b);
    req.thread = Some(&key);
    assert!(matches!(
        resolve_grouping(&snapshot, &req).unwrap().thread,
        ThreadTarget::Create(_)
    ));
}

#[test]
fn thread_precedence_is_named_then_rule_then_the_conversations_default() {
    let snapshot = GroupingSnapshot {
        topics: vec![topic("T1", Some("project:gentbox"), "gentbox")],
        threads: vec![
            thread("H-named", "T1", Some("named")),
            thread("H-rule", "T1", Some("rule-thread")),
            thread("H-default", "T1", None),
        ],
        default_links: vec![link("c1", "T1", "H-default", 0)],
    };
    let topic = topic_ref(json!({ "key": "project:gentbox" }));
    let named = thread_ref(json!({ "key": "named" }));
    let rule = spec("rule-thread", "Rule thread");
    let mut req = request("c1");
    req.topic = Some(&topic);
    req.thread = Some(&named);
    req.rule_thread = Some(&rule);
    assert_eq!(
        existing_thread(&resolve_grouping(&snapshot, &req).unwrap()),
        "H-named"
    );
    req.thread = None;
    assert_eq!(
        existing_thread(&resolve_grouping(&snapshot, &req).unwrap()),
        "H-rule"
    );
    req.rule_thread = None;
    let resolution = resolve_grouping(&snapshot, &req).unwrap();
    assert_eq!(existing_thread(&resolution), "H-default");
    assert!(resolution.uses_default_thread);
}

#[test]
fn the_default_thread_is_implicit_and_created_on_the_conversations_first_message() {
    let resolution = resolve_grouping(&GroupingSnapshot::default(), &request("c1")).unwrap();
    let created = new_thread(&resolution);
    assert!(created.implicit);
    assert_eq!(created.key, None);
    assert_eq!(created.kind.as_deref(), Some("conversation"));
    assert_eq!(created.title, "Fix flaky ingest test");
    assert_eq!(
        created.default_for,
        Some(DefaultFor {
            agent: "claude-code".to_owned(),
            conversation_id: "c1".to_owned()
        })
    );
    assert!(resolution.uses_default_thread);

    let mut untitled = request("c1");
    untitled.conversation_title = None;
    assert_eq!(
        new_thread(&resolve_grouping(&GroupingSnapshot::default(), &untitled).unwrap()).title,
        "gentbox"
    );
    untitled.project = None;
    assert_eq!(
        new_thread(&resolve_grouping(&GroupingSnapshot::default(), &untitled).unwrap()).title,
        "Conversation"
    );
    let mut flagged = request("c1");
    flagged.conversation_title = Some("cargo test -p ingest");
    assert_eq!(
        new_thread(&resolve_grouping(&GroupingSnapshot::default(), &flagged).unwrap()).title,
        "gentbox"
    );
}

#[test]
fn a_conversation_has_one_default_thread_per_topic() {
    // The conversation posts into two topics: each gets its own default thread.
    let snapshot = GroupingSnapshot {
        topics: vec![topic("TA", Some("a"), "a"), topic("TB", Some("b"), "b")],
        threads: vec![thread("HA", "TA", None)],
        default_links: vec![link("c1", "TA", "HA", 0)],
    };
    let in_a = topic_ref(json!({ "key": "a" }));
    let in_b = topic_ref(json!({ "key": "b" }));
    let mut req = request("c1");
    req.topic = Some(&in_a);
    assert_eq!(
        existing_thread(&resolve_grouping(&snapshot, &req).unwrap()),
        "HA"
    );
    req.topic = Some(&in_b);
    let resolution = resolve_grouping(&snapshot, &req).unwrap();
    assert_eq!(existing_topic(&resolution), "TB");
    assert!(
        new_thread(&resolution).implicit,
        "a second implicit thread in the other topic"
    );
}

#[test]
fn a_thread_spans_two_conversations() {
    let snapshot = GroupingSnapshot {
        topics: vec![topic("T1", Some("project:gentbox"), "gentbox")],
        threads: vec![thread("H1", "T1", Some("phase-3-ingest"))],
        default_links: vec![],
    };
    let topic = topic_ref(json!({ "key": "project:gentbox" }));
    let key = thread_ref(json!({ "key": "phase-3-ingest" }));
    for conversation in ["session-a", "session-b"] {
        let mut req = request(conversation);
        req.topic = Some(&topic);
        req.thread = Some(&key);
        let resolution = resolve_grouping(&snapshot, &req).unwrap();
        assert_eq!(existing_topic(&resolution), "T1");
        assert_eq!(existing_thread(&resolution), "H1");
    }
}

#[test]
fn a_message_to_a_closed_thread_or_topic_reopens_it() {
    let mut closed_topic = topic("T1", Some("project:gentbox"), "gentbox");
    closed_topic.status = GroupStatus::Closed;
    let mut closed_thread = thread("H1", "T1", Some("phase-3"));
    closed_thread.status = GroupStatus::Closed;
    closed_thread.closed_by_topic = true;
    let snapshot = GroupingSnapshot {
        topics: vec![closed_topic],
        threads: vec![closed_thread],
        default_links: vec![],
    };
    let key = thread_ref(json!({ "key": "phase-3" }));
    let topic = topic_ref(json!({ "key": "project:gentbox" }));
    let mut req = request("c1");
    req.topic = Some(&topic);
    req.thread = Some(&key);
    let resolution = resolve_grouping(&snapshot, &req).unwrap();
    assert_eq!(resolution.reopens_thread, Some(ThreadId::new("H1")));
    assert_eq!(resolution.reopens_topic, Some(TopicId::new("T1")));

    let by_id = thread_ref(json!({ "id": "H1" }));
    let mut req = request("c1");
    req.thread = Some(&by_id);
    let resolution = resolve_grouping(&snapshot, &req).unwrap();
    assert_eq!(resolution.reopens_thread, Some(ThreadId::new("H1")));
    assert_eq!(resolution.reopens_topic, Some(TopicId::new("T1")));
}

#[test]
fn open_targets_reopen_nothing() {
    let snapshot = GroupingSnapshot {
        topics: vec![topic("T1", Some("k"), "k")],
        threads: vec![thread("H1", "T1", Some("t"))],
        default_links: vec![],
    };
    let topic = topic_ref(json!({ "key": "k" }));
    let key = thread_ref(json!({ "key": "t" }));
    let mut req = request("c1");
    req.topic = Some(&topic);
    req.thread = Some(&key);
    let resolution = resolve_grouping(&snapshot, &req).unwrap();
    assert_eq!(
        (resolution.reopens_topic, resolution.reopens_thread),
        (None, None)
    );
}

#[test]
fn kind_hints_never_change_a_resolution() {
    let snapshot = GroupingSnapshot::default();
    let kinds = [
        "project",
        "category",
        "phase",
        "version",
        "conversation",
        "general",
        "anything",
    ];
    let baseline = {
        let named = topic_ref(json!({ "key": "k" }));
        let key = thread_ref(json!({ "key": "t" }));
        let mut req = request("c1");
        req.topic = Some(&named);
        req.thread = Some(&key);
        resolve_grouping(&snapshot, &req).unwrap()
    };
    for kind in kinds {
        let named = topic_ref(json!({ "key": "k", "kind": kind }));
        let key = thread_ref(json!({ "key": "t", "kind": kind }));
        let mut req = request("c1");
        req.topic = Some(&named);
        req.thread = Some(&key);
        let resolution = resolve_grouping(&snapshot, &req).unwrap();
        assert_eq!(
            (
                new_topic(&resolution).key.clone(),
                new_thread(&resolution).key.clone()
            ),
            (
                new_topic(&baseline).key.clone(),
                new_thread(&baseline).key.clone()
            ),
            "{kind}"
        );
        assert!(!new_thread(&resolution).implicit, "{kind}");
    }
}

// --- creates and takeover ---

fn create_request(value: serde_json::Value) -> ThreadCreate {
    serde_json::from_value(value).unwrap()
}

fn context() -> ThreadCreateContext<'static> {
    ThreadCreateContext {
        agent: "claude-code",
        rule_topic: None,
    }
}

fn project_snapshot(with_implicit_default: bool) -> GroupingSnapshot {
    GroupingSnapshot {
        topics: vec![topic("T1", Some("project:gentbox"), "gentbox")],
        threads: if with_implicit_default {
            vec![thread("H-implicit", "T1", None)]
        } else {
            vec![]
        },
        default_links: if with_implicit_default {
            vec![link("c1", "T1", "H-implicit", 0)]
        } else {
            vec![]
        },
    }
}

#[test]
fn creating_a_topic_is_an_upsert_by_key() {
    let snapshot = GroupingSnapshot {
        topics: vec![topic("T1", Some("project:gentbox"), "gentbox")],
        ..GroupingSnapshot::default()
    };
    let existing: TopicCreate =
        serde_json::from_value(json!({ "key": "project:gentbox", "title": "Other title" }))
            .unwrap();
    assert_eq!(
        create_topic(&snapshot, &existing),
        Ok(TopicCreateOutcome::Existing(TopicId::new("T1")))
    );
    let fresh: TopicCreate = serde_json::from_value(json!({ "key": "release-1.4", "title": "Release 1.4", "kind": "other", "style": { "color": "teal" } })).unwrap();
    let TopicCreateOutcome::Created(created) = create_topic(&snapshot, &fresh).unwrap() else {
        panic!("expected 201")
    };
    assert_eq!(created.key.as_deref(), Some("release-1.4"));
    assert_eq!(created.title, "Release 1.4");
    assert!(created.style.is_some());
}

#[test]
fn explicit_creates_with_a_flagged_title_are_refused() {
    let snapshot = GroupingSnapshot::default();
    let topic: TopicCreate =
        serde_json::from_value(json!({ "key": "k", "title": "git push origin main" })).unwrap();
    assert_eq!(
        create_topic(&snapshot, &topic),
        Err(GroupingError::TitleLooksLikeCommand)
    );
    let thread = create_request(
        json!({ "key": "t", "title": "run ./scripts/test.sh", "topic": { "key": "k" } }),
    );
    assert_eq!(
        create_thread(&snapshot, &thread, &context()),
        Err(GroupingError::TitleLooksLikeCommand)
    );
}

#[test]
fn creating_a_thread_without_a_conversation_is_an_ordinary_thread() {
    let snapshot = project_snapshot(true);
    let request = create_request(
        json!({ "topic": { "key": "project:gentbox" }, "key": "phase-3", "title": "Phase 3", "kind": "phase" }),
    );
    let outcome = create_thread(&snapshot, &request, &context()).unwrap();
    assert_eq!(outcome.topic, TopicTarget::Existing(TopicId::new("T1")));
    let ThreadCreateAction::Created { thread, takeover } = outcome.action else {
        panic!("expected a create")
    };
    assert_eq!((thread.default_for, takeover), (None, None));
}

#[test]
fn creating_a_thread_upserts_by_key_within_the_topic() {
    let mut snapshot = project_snapshot(false);
    snapshot.threads.push(thread("H1", "T1", Some("phase-3")));
    let request = create_request(
        json!({ "topic": { "key": "project:gentbox" }, "key": "phase-3", "title": "Whatever" }),
    );
    let outcome = create_thread(&snapshot, &request, &context()).unwrap();
    assert_eq!(
        outcome.action,
        ThreadCreateAction::Existing(ThreadId::new("H1"))
    );

    snapshot.topics.push(topic("T2", Some("other"), "other"));
    let other_topic = create_request(
        json!({ "topic": { "key": "other" }, "key": "phase-3", "title": "Phase 3" }),
    );
    assert!(matches!(
        create_thread(&snapshot, &other_topic, &context())
            .unwrap()
            .action,
        ThreadCreateAction::Created { .. }
    ));
}

#[test]
fn a_conversation_thread_takes_over_the_implicit_default_thread() {
    let mut snapshot = project_snapshot(true);
    snapshot.threads[0].muted = true;
    let request = create_request(json!({
        "topic": { "key": "project:gentbox" }, "key": "session:web-release-notes",
        "title": "Release notes page", "kind": "conversation", "conversation": { "id": "c1" }
    }));
    let outcome = create_thread(&snapshot, &request, &context()).unwrap();
    let ThreadCreateAction::Created { thread, takeover } = outcome.action else {
        panic!("expected a create")
    };
    assert_eq!(
        thread.default_for,
        Some(DefaultFor {
            agent: "claude-code".to_owned(),
            conversation_id: "c1".to_owned()
        })
    );
    assert_eq!(
        takeover,
        Some(Takeover {
            replaces: ThreadId::new("H-implicit"),
            inherits_mute: true
        })
    );
}

#[test]
fn takeover_happens_only_once_and_never_by_an_existing_key() {
    // Once the default thread is explicit, a second conversation thread is ordinary.
    let mut snapshot = project_snapshot(false);
    let mut explicit = thread("H-explicit", "T1", Some("session:first"));
    explicit.kind = Some("conversation".to_owned());
    snapshot.threads.push(explicit);
    snapshot
        .default_links
        .push(link("c1", "T1", "H-explicit", 0));
    let second = create_request(json!({
        "topic": { "key": "project:gentbox" }, "key": "session:second", "title": "Second",
        "kind": "conversation", "conversation": { "id": "c1" }
    }));
    let ThreadCreateAction::Created { thread, takeover } =
        create_thread(&snapshot, &second, &context())
            .unwrap()
            .action
    else {
        panic!("expected a create")
    };
    assert_eq!((thread.default_for, takeover), (None, None));

    // Repeating the first create is idempotent: the key exists, nothing is taken over again.
    let repeat = create_request(json!({
        "topic": { "key": "project:gentbox" }, "key": "session:first", "title": "First",
        "kind": "conversation", "conversation": { "id": "c1" }
    }));
    assert_eq!(
        create_thread(&snapshot, &repeat, &context())
            .unwrap()
            .action,
        ThreadCreateAction::Existing(ThreadId::new("H-explicit"))
    );
}

#[test]
fn a_conversation_thread_with_no_implicit_thread_yet_becomes_the_default_without_a_takeover() {
    let snapshot = project_snapshot(false);
    let request = create_request(json!({
        "topic": { "key": "project:gentbox" }, "key": "session:new", "title": "New",
        "kind": "conversation", "conversation": { "id": "c1" }
    }));
    let ThreadCreateAction::Created { thread, takeover } =
        create_thread(&snapshot, &request, &context())
            .unwrap()
            .action
    else {
        panic!("expected a create")
    };
    assert!(thread.default_for.is_some());
    assert_eq!(takeover, None);
}

#[test]
fn only_the_conversation_kind_takes_over() {
    let snapshot = project_snapshot(true);
    for kind in ["phase", "version", "other"] {
        let request = create_request(json!({
            "topic": { "key": "project:gentbox" }, "key": "t", "title": "T",
            "kind": kind, "conversation": { "id": "c1" }
        }));
        let ThreadCreateAction::Created { thread, takeover } =
            create_thread(&snapshot, &request, &context())
                .unwrap()
                .action
        else {
            panic!("expected a create")
        };
        assert_eq!((thread.default_for, takeover), (None, None), "{kind}");
    }
}

#[test]
fn a_takeover_in_another_topic_leaves_this_topics_implicit_thread_alone() {
    let mut snapshot = project_snapshot(true);
    snapshot.topics.push(topic("T2", Some("other"), "other"));
    let request = create_request(json!({
        "topic": { "key": "other" }, "key": "session:x", "title": "X",
        "kind": "conversation", "conversation": { "id": "c1" }
    }));
    let ThreadCreateAction::Created { takeover, .. } =
        create_thread(&snapshot, &request, &context())
            .unwrap()
            .action
    else {
        panic!("expected a create")
    };
    assert_eq!(
        takeover, None,
        "the conversation's implicit thread is in another topic"
    );
}

#[test]
fn a_thread_create_without_a_topic_resolves_it_like_a_message() {
    let snapshot = GroupingSnapshot {
        topics: vec![topic("T1", Some("project:gentbox"), "gentbox")],
        threads: vec![thread("H-implicit", "T1", None)],
        default_links: vec![link("c1", "T1", "H-implicit", 0)],
    };
    let request = create_request(
        json!({ "key": "session:x", "title": "X", "kind": "conversation", "conversation": { "id": "c1" } }),
    );
    let outcome = create_thread(&snapshot, &request, &context()).unwrap();
    assert_eq!(
        outcome.topic,
        TopicTarget::Existing(TopicId::new("T1")),
        "latest default thread's topic"
    );
    let ThreadCreateAction::Created { takeover, .. } = outcome.action else {
        panic!("expected a create")
    };
    assert!(takeover.is_some());

    let nowhere = create_request(json!({ "key": "t", "title": "T" }));
    let outcome = create_thread(&GroupingSnapshot::default(), &nowhere, &context()).unwrap();
    assert!(matches!(outcome.topic, TopicTarget::Create(ref general) if general.implicit));
}

// --- rule templates ---

fn vars(project: Option<&str>) -> TemplateVars {
    TemplateVars {
        project: project.map(str::to_owned),
        agent: "Claude Code".to_owned(),
        conversation: Some("Fix flaky ingest test".to_owned()),
    }
}

fn template(key: &str, title: Option<&str>, kind: Option<&str>) -> GroupTemplate {
    GroupTemplate {
        key: key.to_owned(),
        title: title.map(str::to_owned),
        kind: kind.map(str::to_owned),
    }
}

#[test]
fn templates_fill_project_agent_and_conversation() {
    let rendered = template("project:{project}", Some("{project}"), Some("project"))
        .render(&vars(Some("gentbox")))
        .unwrap();
    assert_eq!(
        rendered,
        GroupSpec {
            key: "project:gentbox".to_owned(),
            title: "gentbox".to_owned(),
            kind: Some("project".to_owned())
        }
    );
    let by_agent = template("agent:{agent}", Some("{agent} sessions"), None)
        .render(&vars(None))
        .unwrap();
    assert_eq!(by_agent.key, "agent:claude-code");
    assert_eq!(by_agent.title, "Claude Code sessions");
    let by_conversation = template("session:{conversation}", Some("{conversation}"), None)
        .render(&vars(None))
        .unwrap();
    assert_eq!(by_conversation.key, "session:fix-flaky-ingest-test");
    assert_eq!(by_conversation.title, "Fix flaky ingest test");
}

#[test]
fn template_keys_are_slugged_and_titles_keep_the_original() {
    let rendered = template("project:{project}", Some("{project}"), None)
        .render(&vars(Some("My App_2.0")))
        .unwrap();
    assert_eq!(rendered.key, "project:my-app_2.0");
    assert_eq!(rendered.title, "My App_2.0");
    assert!(is_valid_group_key(&rendered.key));
}

#[test]
fn a_template_with_a_missing_value_assigns_nothing() {
    assert_eq!(
        template("project:{project}", None, None).render(&vars(None)),
        None
    );
    assert_eq!(
        template("project:{project}", None, None).render(&vars(Some("日本語"))),
        None,
        "nothing left after slugging"
    );
    let long = "x".repeat(200);
    let truncated = template("project:{project}", None, None)
        .render(&vars(Some(&long)))
        .unwrap();
    assert_eq!(
        truncated.key.chars().count(),
        96,
        "long keys are cut, not dropped"
    );
    assert!(truncated.key.starts_with("project:xxx"));
    assert!(is_valid_group_key(&truncated.key));
}

#[test]
fn a_rendered_kind_is_reduced_to_kind_characters() {
    let rendered = template("k-{agent}", None, Some("{agent}"))
        .render(&vars(None))
        .unwrap();
    assert_eq!(rendered.kind.as_deref(), Some("claude-code"));
    let long = TemplateVars {
        agent: "A".repeat(80),
        ..vars(None)
    };
    let kind = template("k", None, Some("{agent}"))
        .render(&long)
        .unwrap()
        .kind
        .unwrap();
    assert_eq!(kind.chars().count(), 32);
}

#[test]
fn a_flagged_template_title_falls_back_to_the_key_in_words() {
    let rendered = template("project:{project}", Some("{project}"), None)
        .render(&vars(Some("cargo-test --all")))
        .unwrap();
    assert!(
        rendered
            .title
            .chars()
            .next()
            .is_some_and(char::is_uppercase)
    );
    assert!(gentbox_core::title_guard::is_plain_words(&rendered.title));
    let no_title = template("phase-3-ingest", None, None)
        .render(&vars(None))
        .unwrap();
    assert_eq!(no_title.title, "Phase 3 ingest");
}

#[test]
fn group_keys_follow_the_schema_pattern() {
    for (key, valid) in [
        ("project:gentbox", true),
        ("release-1.4", true),
        ("a", true),
        ("repo:owner/name", true),
        ("", false),
        ("-leading", false),
        (".leading", false),
        ("Upper", false),
        ("has space", false),
        ("under_score", true),
    ] {
        assert_eq!(is_valid_group_key(key), valid, "{key:?}");
    }
    assert!(!is_valid_group_key(&"a".repeat(97)));
    assert!(is_valid_group_key(&"a".repeat(96)));
}

#[test]
fn rule_templates_flow_through_the_prepared_message_into_resolution() {
    use gentbox_core::classify::{BuiltInRules, RuleSet};
    use gentbox_core::prepare::{IngestSettings, prepare_message};
    let rules = RuleSet::parse(
        r#"
        [[rule]]
        name = "One topic per project"
        match = { source = "claude-code" }
        topic = { key = "project:{project}", title = "{project}", kind = "project" }
        thread = { key = "conversation-{conversation}", title = "{conversation}" }
        "#,
    )
    .unwrap();
    let prepared = prepare_message(
        support::input(json!({ "conversation": { "id": "c1", "cwd": "/Users/me/Developer/gentbox", "title": "Fix flaky ingest test" } })),
        &rules,
        BuiltInRules::load(),
        IngestSettings::default(),
    )
    .unwrap();
    let resolution =
        resolve_grouping(&GroupingSnapshot::default(), &prepared.grouping_request()).unwrap();
    assert_eq!(
        new_topic(&resolution).key.as_deref(),
        Some("project:gentbox")
    );
    assert_eq!(
        new_thread(&resolution).key.as_deref(),
        Some("conversation-fix-flaky-ingest-test")
    );
    assert_eq!(new_thread(&resolution).title, "Fix flaky ingest test");
}

#[test]
fn project_name_is_the_last_directory() {
    for (cwd, expected) in [
        (Some("/Users/me/Developer/gentbox"), Some("gentbox")),
        (Some("/Users/me/Developer/gentbox/"), Some("gentbox")),
        (Some("/"), None),
        (None, None),
    ] {
        let conversation = Conversation {
            id: "c".to_owned(),
            title: None,
            cwd: cwd.map(str::to_owned),
            focus: None,
        };
        assert_eq!(conversation.project_name(), expected, "{cwd:?}");
    }
}

// --- mute ---

#[test]
fn every_combination_muted_by_topic_by_thread_and_not_muted() {
    let policy = Policy::default();
    for (message_type, urgent, blocking) in valid_combinations() {
        let message = awaiting(message_type, urgent, blocking);
        let plain_topic = topic("T1", Some("k"), "k");
        let plain_thread = thread("H1", "T1", Some("t"));
        let muted_topic = Topic {
            muted: true,
            ..plain_topic.clone()
        };
        let muted_thread = Thread {
            muted: true,
            ..plain_thread.clone()
        };
        let cases = [
            ("not muted", &plain_topic, &plain_thread, false),
            ("muted by topic", &muted_topic, &plain_thread, true),
            ("muted by thread", &plain_topic, &muted_thread, true),
            ("muted by both", &muted_topic, &muted_thread, true),
        ];
        for (label, topic, thread, expected_muted) in cases {
            assert_eq!(
                is_effectively_muted(topic, thread),
                expected_muted,
                "{label}"
            );
            let mut context = calm_context();
            context.is_muted = is_effectively_muted(topic, thread);
            let plan = decide(&message, &context, &policy);
            let calm = decide(&message, &calm_context(), &policy);
            let id = format!("{message_type:?} urgent={urgent} {blocking:?} {label}");
            if expected_muted && !urgent {
                assert_eq!(plan.mac, MacSurface::InboxOnly, "{id}");
                assert!(!plan.should_wait, "{id}: muted senders must not hang");
                assert!(!plan.sound, "{id}");
                assert_eq!(plan.reping, None, "{id}");
            } else {
                assert_eq!(
                    plan, calm,
                    "{id}: urgent and not-muted messages are unaffected"
                );
            }
        }
    }
}
