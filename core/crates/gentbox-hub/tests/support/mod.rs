//! A hub on a temp state folder with a manual clock and a delegate that
//! records every event.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

use gentbox_core::ids::MessageId;
use gentbox_core::message::MessageInput;
use gentbox_core::response::{ActionResponse, AnsweredVia, IngestResponse, ResponseStatus};
use gentbox_hub::{
    ActionRequest, Hub, HubConfig, HubDelegate, HubEvent, HubHandle, IngestOutcome, IngestStatus,
    ManualClock,
};
use serde_json::{Value, json};
use tempfile::TempDir;
use time::OffsetDateTime;
use time::macros::datetime;

pub const START: OffsetDateTime = datetime!(2026-10-24 12:00:00 UTC);

/// Records every event the hub dispatches.
#[derive(Debug, Default)]
pub struct Recorder {
    events: Mutex<Vec<HubEvent>>,
}

impl HubDelegate for Recorder {
    fn on_event(&self, event: HubEvent) {
        self.events.lock().unwrap().push(event);
    }
}

impl Recorder {
    pub fn events(&self) -> Vec<HubEvent> {
        self.events.lock().unwrap().clone()
    }

    pub fn clear(&self) {
        self.events.lock().unwrap().clear();
    }
}

pub struct TestHub {
    pub hub: Option<Hub>,
    pub handle: HubHandle,
    pub clock: ManualClock,
    pub recorder: Arc<Recorder>,
    pub directory: TempDir,
}

impl TestHub {
    pub fn start() -> Self {
        Self::start_with(|_| {})
    }

    pub fn start_with(configure: impl FnOnce(&mut HubConfig)) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let clock = ManualClock::new(START);
        let mut config = HubConfig::new(directory.path());
        config.clock = Arc::new(clock.clone());
        configure(&mut config);
        let recorder = Arc::new(Recorder::default());
        let hub = Hub::start(config, recorder.clone(), None).unwrap();
        let handle = hub.handle();
        Self {
            hub: Some(hub),
            handle,
            clock,
            recorder,
            directory,
        }
    }

    pub async fn post(&self, fields: Value) -> IngestOutcome {
        self.handle.ingest(message(fields)).await.unwrap()
    }

    /// Posts and expects a new message.
    pub async fn create(&self, fields: Value) -> IngestResponse {
        let outcome = self.post(fields).await;
        assert_eq!(outcome.status, IngestStatus::Created, "{outcome:?}");
        outcome.response
    }

    /// Moves the clock and fires what fell due. The hub first catches up on
    /// what already happened (a poll that ended is stamped with the time the
    /// hub hears of it).
    pub async fn advance(&self, step: time::Duration) {
        self.handle.run_due_timers().await.unwrap();
        self.clock.advance(step);
        self.handle.run_due_timers().await.unwrap();
    }

    pub async fn poll(&self, id: &MessageId, wait: StdDuration) -> ActionResponse {
        self.handle
            .wait_for_response(id.clone(), wait)
            .await
            .unwrap()
    }

    pub async fn status_of(&self, id: &MessageId) -> ResponseStatus {
        self.poll(id, StdDuration::ZERO).await.status
    }

    pub async fn press(
        &self,
        id: &MessageId,
        action_id: &str,
    ) -> Result<(), gentbox_hub::HubError> {
        self.handle
            .take_action(
                id.clone(),
                ActionRequest {
                    action_id: action_id.to_owned(),
                    value: None,
                    via: AnsweredVia::Mac,
                },
            )
            .await
            .map(|_| ())
    }

    pub async fn stop(mut self) {
        if let Some(hub) = self.hub.take() {
            hub.shutdown().await;
        }
    }
}

/// A valid message with the given fields merged in.
pub fn message(fields: Value) -> MessageInput {
    let mut base = json!({
        "version": 1,
        "source": { "agent": "claude-code", "displayName": "Claude Code" },
        "conversation": { "id": "conversation-1", "cwd": "/Users/me/Developer/gentbox" },
        "title": "Ingest tests pass"
    });
    merge(&mut base, fields);
    serde_json::from_value(base).unwrap()
}

pub fn merge(base: &mut Value, fields: Value) {
    if let (Some(target), Value::Object(fields)) = (base.as_object_mut(), fields) {
        for (key, value) in fields {
            target.insert(key, value);
        }
    }
}

/// A blocking permission prompt whose sender waits.
pub fn blocking_prompt() -> Value {
    json!({
        "type": "decision_required",
        "blocking": "blocking",
        "title": "Allow Bash to run the ingest tests?",
        "awaitsResponse": true,
        "actions": [
            { "id": "allow", "label": "Allow", "kind": "approve" },
            { "id": "deny", "label": "Deny", "kind": "deny" },
            { "id": "other", "label": "Other…", "kind": "reply" }
        ]
    })
}

/// A deferrable question whose sender waits.
pub fn deferrable_question() -> Value {
    json!({
        "type": "decision_required",
        "blocking": "deferrable",
        "title": "Which push channel should the relay use?",
        "awaitsResponse": true,
        "actions": [
            { "id": "apns", "label": "APNs", "kind": "choice", "value": "apns" },
            { "id": "ntfy", "label": "ntfy", "kind": "choice", "value": "ntfy" }
        ]
    })
}

/// An action_required message with no sender waiting.
pub fn action_without_waiter() -> Value {
    json!({
        "type": "action_required",
        "blocking": "blocking",
        "title": "Sign in to npm to publish the CLI",
        "steps": [{ "text": "Sign in to npm", "detail": "```bash\nnpm login\n```" }]
    })
}

pub fn count<F>(events: &[HubEvent], matches: F) -> usize
where
    F: Fn(&HubEvent) -> bool,
{
    events.iter().filter(|event| matches(event)).count()
}

pub fn id_of(response: &IngestResponse) -> MessageId {
    response.id.clone()
}
