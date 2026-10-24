//! The FFI surface, end to end without Swift: a hub started as the Mac app
//! starts it, messages posted over the real loopback API as `curl` would,
//! and the grouped inbox, the group commands and the delegate as Swift sees
//! them.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use gentbox_ffi::{
    GentboxError, GentboxHub, GroupKind, HubDelegate, HubEvent, HubOptions, Inbox, PresenceState,
    PresenceUpdate,
};
use serde_json::{Value, json};
use tokio::runtime::Runtime;

/// Records events and the thread they arrived on.
#[derive(Default)]
struct Recorder {
    events: Mutex<Vec<(HubEvent, Option<String>)>>,
}

impl HubDelegate for Recorder {
    fn on_event(&self, event: HubEvent) {
        let thread = std::thread::current().name().map(str::to_owned);
        self.events.lock().unwrap().push((event, thread));
    }
}

impl Recorder {
    fn delivered_titles(&self) -> Vec<String> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|(event, _)| match event {
                HubEvent::Delivered { message, .. } => Some(message.title.clone()),
                _ => None,
            })
            .collect()
    }
}

/// A hub in a temporary state folder, and a runtime for the test's own
/// async calls (the hub must be started and dropped outside any runtime, as
/// the app does from its main thread).
struct Fixture {
    hub: Arc<GentboxHub>,
    recorder: Arc<Recorder>,
    runtime: Runtime,
    client: reqwest::Client,
    token: String,
    _state: tempfile::TempDir,
}

impl Fixture {
    fn start() -> Self {
        let state = tempfile::tempdir().unwrap();
        let recorder = Arc::new(Recorder::default());
        let hub = GentboxHub::start(options(state.path()), recorder.clone()).unwrap();
        let token = std::fs::read_to_string(state.path().join("API/token"))
            .unwrap()
            .trim()
            .to_owned();
        Self {
            hub,
            recorder,
            runtime: tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap(),
            client: reqwest::Client::new(),
            token,
            _state: state,
        }
    }

    /// `POST /v1/messages` over TCP; returns the 201 body.
    fn post(&self, fields: Value) -> Value {
        let mut body = json!({
            "version": 1,
            "source": { "agent": "claude-code", "displayName": "Claude Code" },
            "conversation": { "id": "session-1", "cwd": "/Users/me/Developer/gentbox" },
            "title": "Ingest tests pass"
        });
        if let (Some(target), Value::Object(fields)) = (body.as_object_mut(), fields) {
            target.extend(fields);
        }
        let url = format!("http://127.0.0.1:{}/v1/messages", self.hub.port());
        self.runtime.block_on(async {
            let response = self
                .client
                .post(url)
                .bearer_auth(&self.token)
                .json(&body)
                .send()
                .await
                .unwrap();
            assert_eq!(response.status().as_u16(), 201);
            response.json().await.unwrap()
        })
    }

    fn inbox(&self) -> Inbox {
        self.runtime.block_on(self.hub.inbox()).unwrap()
    }

    fn block_on<T>(&self, future: impl Future<Output = T>) -> T {
        self.runtime.block_on(future)
    }
}

fn options(state_dir: &std::path::Path) -> HubOptions {
    HubOptions {
        state_dir: state_dir.display().to_string(),
        home_dir: None,
    }
}

fn topic_titles(inbox: &Inbox) -> Vec<&str> {
    inbox
        .topics
        .iter()
        .map(|topic| topic.title.as_str())
        .collect()
}

#[test]
fn a_simple_conversation_shows_as_general_then_its_thread() {
    let fixture = Fixture::start();
    fixture.post(json!({
        "type": "task_completed",
        "conversation": { "id": "session-1", "title": "Refactor the store" }
    }));
    let inbox = fixture.inbox();
    assert_eq!(topic_titles(&inbox), vec!["General"]);
    let general = &inbox.topics[0];
    assert!(general.is_general);
    assert_eq!(general.threads.len(), 1);
    let thread = &general.threads[0];
    assert!(thread.is_implicit);
    assert_eq!(thread.title, "Refactor the store");
    assert_eq!(thread.messages[0].title, "Ingest tests pass");
    assert_eq!(thread.messages[0].agent_name, "Claude Code");
    assert!(inbox.next_up.is_none());
}

#[test]
fn groups_follow_topic_then_thread_with_the_blocked_on_message_lifted_out() {
    let fixture = Fixture::start();
    let phase = json!({ "key": "phase-3", "title": "Phase 3" });
    fixture.post(json!({
        "topic": { "key": "gentbox", "title": "Gentbox" }, "thread": phase,
        "type": "task_completed", "title": "Store migrations run on start"
    }));
    fixture.post(json!({
        "topic": { "key": "infra", "title": "Infrastructure" },
        "thread": { "key": "certs", "title": "Rotate staging certificates" },
        "type": "task_completed", "urgent": true, "title": "Staging certificates rotated"
    }));
    fixture.post(json!({
        "topic": { "key": "gentbox" }, "thread": { "key": "phase-3" },
        "type": "decision_required", "blocking": "blocking", "awaitsResponse": true,
        "title": "Allow Bash to run the ingest tests?",
        "actions": [
            { "id": "allow", "label": "Allow", "kind": "approve" },
            { "id": "deny", "label": "Deny", "kind": "deny" }
        ]
    }));
    fixture.post(json!({
        // Another conversation: in the same one it would fold into the
        // prompt that has a live waiter (architecture §7).
        "conversation": { "id": "session-2" },
        "topic": { "key": "gentbox" }, "thread": { "key": "v1-1", "title": "v1.1" },
        "type": "decision_required", "blocking": "deferrable",
        "title": "Which title for the 1.1 release notes?"
    }));

    let inbox = fixture.inbox();
    let next_up = inbox.next_up.as_ref().unwrap();
    assert_eq!(next_up.title, "Allow Bash to run the ingest tests?");
    assert!(next_up.has_live_waiter);
    assert!(
        next_up
            .offered_actions
            .iter()
            .any(|action| action.id == "allow")
    );
    // Urgent first, so Infrastructure leads although it is older.
    assert_eq!(topic_titles(&inbox), vec!["Infrastructure", "Gentbox"]);
    let gentbox = &inbox.topics[1];
    let threads: Vec<_> = gentbox
        .threads
        .iter()
        .map(|thread| thread.title.as_str())
        .collect();
    // v1.1 waits on you (deferrable); Phase 3 has only a completion left.
    assert_eq!(threads, vec!["v1.1", "Phase 3"]);
    assert_eq!(inbox.waiting_count, 2);
    let targets: Vec<_> = inbox
        .move_targets
        .iter()
        .map(|topic| topic.title.as_str())
        .collect();
    assert_eq!(targets, vec!["Gentbox", "Infrastructure"]);
}

#[test]
fn muting_a_topic_or_a_thread_keeps_it_listed_and_names_it_in_the_footer() {
    let fixture = Fixture::start();
    fixture.post(json!({
        "topic": { "key": "deps", "title": "Dependency upgrades" },
        "thread": { "key": "tokio", "title": "Tokio 1.55" },
        "type": "task_completed", "title": "Tokio upgraded; 212 tests pass"
    }));
    fixture.post(json!({
        "topic": { "key": "web", "title": "Web" },
        "thread": { "key": "search", "title": "Search filters" },
        "type": "task_completed", "title": "Search filters persist in the URL"
    }));
    let inbox = fixture.inbox();
    let deps = inbox
        .topics
        .iter()
        .find(|topic| topic.title == "Dependency upgrades")
        .unwrap();
    let web_thread = inbox
        .topics
        .iter()
        .find(|topic| topic.title == "Web")
        .unwrap()
        .threads[0]
        .id
        .clone();
    fixture
        .block_on(fixture.hub.mute_topic(deps.id.clone(), true))
        .unwrap();
    fixture
        .block_on(fixture.hub.mute_thread(web_thread.clone(), true))
        .unwrap();

    let inbox = fixture.inbox();
    assert_eq!(inbox.topics.len(), 2);
    assert!(inbox.topics.iter().any(|topic| topic.is_muted));
    let muted: Vec<_> = inbox
        .muted
        .iter()
        .map(|group| (group.kind, group.title.as_str()))
        .collect();
    assert_eq!(
        muted,
        vec![
            (GroupKind::Topic, "Dependency upgrades"),
            (GroupKind::Thread, "Search filters"),
        ]
    );

    fixture
        .block_on(fixture.hub.mute_thread(web_thread, false))
        .unwrap();
    assert_eq!(fixture.inbox().muted.len(), 1);
}

#[test]
fn closing_moves_a_group_to_the_closed_section_and_reopening_brings_it_back() {
    let fixture = Fixture::start();
    fixture.post(json!({
        "topic": { "key": "gentbox", "title": "Gentbox" },
        "thread": { "key": "v1-0", "title": "v1.0" },
        "type": "task_completed", "title": "v1.0 tagged"
    }));
    let thread_id = fixture.inbox().topics[0].threads[0].id.clone();
    fixture
        .block_on(fixture.hub.close_thread(thread_id.clone(), false))
        .unwrap();
    let inbox = fixture.inbox();
    assert!(inbox.topics.is_empty());
    assert_eq!(inbox.closed.len(), 1);
    assert_eq!(inbox.closed[0].kind, GroupKind::Thread);
    assert_eq!(inbox.closed[0].topic_title.as_deref(), Some("Gentbox"));

    fixture
        .block_on(fixture.hub.reopen_thread(thread_id))
        .unwrap();
    let inbox = fixture.inbox();
    assert!(inbox.closed.is_empty());
    assert_eq!(inbox.topics[0].threads[0].messages.len(), 1);

    let topic_id = inbox.topics[0].id.clone();
    fixture
        .block_on(fixture.hub.close_topic(topic_id, true))
        .unwrap();
    let inbox = fixture.inbox();
    // The topic's row stands for its threads.
    let closed: Vec<_> = inbox.closed.iter().map(|group| group.kind).collect();
    assert_eq!(closed, vec![GroupKind::Topic]);
}

#[test]
fn you_move_a_thread_to_another_topic() {
    let fixture = Fixture::start();
    fixture.post(json!({
        "topic": { "key": "infra", "title": "Infrastructure" },
        "thread": { "key": "certs", "title": "Rotate staging certificates" },
        "type": "task_completed", "title": "Staging certificates rotated"
    }));
    fixture.post(json!({
        "source": { "agent": "codex" }, "conversation": { "id": "codex-1" },
        "type": "task_completed", "title": "Proxy config reloaded"
    }));
    let inbox = fixture.inbox();
    let infra = inbox
        .topics
        .iter()
        .find(|topic| topic.title == "Infrastructure")
        .unwrap();
    let general_thread = inbox
        .topics
        .iter()
        .find(|topic| topic.is_general)
        .unwrap()
        .threads[0]
        .id
        .clone();
    fixture
        .block_on(fixture.hub.move_thread(general_thread, infra.id.clone()))
        .unwrap();
    let inbox = fixture.inbox();
    assert_eq!(topic_titles(&inbox), vec!["Infrastructure"]);
    assert_eq!(inbox.topics[0].threads.len(), 2);
}

#[test]
fn answers_close_the_message_and_a_second_answer_is_already_closed() {
    let fixture = Fixture::start();
    let created = fixture.post(json!({
        "type": "decision_required", "blocking": "blocking", "awaitsResponse": true,
        "title": "Allow editing cart.ts?",
        "actions": [
            { "id": "allow", "label": "Allow", "kind": "approve" },
            { "id": "deny", "label": "Deny", "kind": "deny" }
        ]
    }));
    let id = created["id"].as_str().unwrap().to_owned();
    fixture
        .block_on(
            fixture
                .hub
                .take_action(id.clone(), "allow".to_owned(), None),
        )
        .unwrap();
    assert!(fixture.inbox().next_up.is_none());
    let second = fixture.block_on(fixture.hub.take_action(id, "deny".to_owned(), None));
    assert!(matches!(second, Err(GentboxError::AlreadyClosed { .. })));
}

#[test]
fn the_delegate_hears_deliveries_on_the_hub_thread() {
    let fixture = Fixture::start();
    fixture.post(json!({ "type": "task_completed", "title": "Docs build" }));
    assert_eq!(fixture.recorder.delivered_titles(), vec!["Docs build"]);
    let events = fixture.recorder.events.lock().unwrap();
    assert!(
        events
            .iter()
            .all(|(_, thread)| thread.as_deref() == Some("gentbox-hub"))
    );
}

#[test]
fn presence_updates_reach_the_hub() {
    let fixture = Fixture::start();
    let update = PresenceUpdate {
        state: PresenceState::Locked,
        frontmost_bundle_id: Some("com.mitchellh.ghostty".to_owned()),
    };
    fixture
        .block_on(fixture.hub.update_presence(update))
        .unwrap();
    fixture.block_on(fixture.hub.update_focus(true)).unwrap();
}

#[test]
fn a_second_hub_on_the_same_folder_is_refused_and_stop_removes_the_port() {
    let state = tempfile::tempdir().unwrap();
    let first = GentboxHub::start(options(state.path()), Arc::new(Recorder::default())).unwrap();
    let second = GentboxHub::start(options(state.path()), Arc::new(Recorder::default()));
    assert!(matches!(
        second,
        Err(GentboxError::AnotherHubRunning { .. })
    ));
    assert!(state.path().join("API/port").exists());

    first.stop();
    assert!(!state.path().join("API/port").exists());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let after = runtime
        .block_on(async { tokio::time::timeout(Duration::from_secs(5), first.inbox()).await });
    assert!(matches!(after, Ok(Err(GentboxError::Stopped))));
    drop(first);
    // The lock is released: a new hub starts on the same folder.
    let again = GentboxHub::start(options(state.path()), Arc::new(Recorder::default()));
    assert!(again.is_ok());
}
