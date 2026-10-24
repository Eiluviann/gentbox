//! Messages through the real router: every schema example, long-polls woken
//! by answers (injected through the hub handle, as the UI does), rescinds and
//! Defer, the title guard, action plans and the `gone` status.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::time::Duration;

use gentbox_core::ids::MessageId;
use gentbox_core::lifecycle::LifecycleState;
use gentbox_core::response::AnsweredVia;
use gentbox_hub::{ActionRequest, HubError};
use gentbox_ingest::ApiError;
use serde_json::{Value, json};
use support::{Server, blocking_prompt, example, id_of};

async fn answer(server: &Server, id: &str, action_id: &str) -> Result<(), HubError> {
    server
        .handle
        .take_action(
            MessageId::new(id),
            ActionRequest {
                action_id: action_id.to_owned(),
                value: None,
                via: AnsweredVia::Mac,
            },
        )
        .await
        .map(|_| ())
}

fn spawn_poll(server: &Server, id: &str, wait_seconds: u64) -> tokio::task::JoinHandle<Value> {
    let client = server.client.clone();
    let url = server.url(&format!("/v1/messages/{id}/response?wait={wait_seconds}"));
    let token = server.token.clone();
    tokio::spawn(async move {
        client
            .get(url)
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    })
}

#[tokio::test]
async fn every_message_example_goes_through_the_router_and_is_stored() {
    let server = Server::start().await;
    // decision-blocking names a topic by id; give it one that exists.
    let topic = server
        .send_for("POST", "/v1/topics", Some(example("topic-create.json")))
        .await;
    assert_eq!(topic.status, 201);
    let cases = [
        (
            "decision-blocking.json",
            "decision_required",
            "blocking",
            "open",
        ),
        (
            "decision-nonblocking.json",
            "decision_required",
            "non_blocking",
            "open",
        ),
        (
            "action-deferrable.json",
            "action_required",
            "deferrable",
            "open",
        ),
        (
            "task-completed.json",
            "task_completed",
            "non_blocking",
            "open",
        ),
        ("generic.json", "generic", "non_blocking", "acknowledged"),
        (
            "undeclared-idle.json",
            "generic",
            "non_blocking",
            "acknowledged",
        ),
        (
            "message-inline-topic.json",
            "decision_required",
            "deferrable",
            "open",
        ),
        (
            "message-thread-session-a.json",
            "decision_required",
            "blocking",
            "open",
        ),
        (
            "message-thread-session-b.json",
            "task_completed",
            "non_blocking",
            "open",
        ),
    ];
    let mut session_threads = Vec::new();
    for (name, message_type, blocking, state) in cases {
        let mut body = example(name);
        if name == "decision-blocking.json" {
            body["topic"]["id"] = topic.body["id"].clone();
        }
        let reply = server.send("POST", "/v1/messages", Some(body)).await;
        assert_eq!(reply.status, 201, "{name}: {reply:?}");
        assert_eq!(reply.body["type"], message_type, "{name}");
        assert_eq!(reply.body["blocking"], blocking, "{name}");
        assert_eq!(reply.body["state"], state, "{name}");
        for field in ["id", "topicId", "threadId"] {
            assert_eq!(
                reply.body[field].as_str().map(str::len),
                Some(26),
                "{name} {field}"
            );
        }

        let stored = server
            .handle
            .message(MessageId::new(id_of(&reply)))
            .await
            .unwrap();
        assert_eq!(stored.message_type.as_str(), message_type, "{name}");
        assert_eq!(stored.blocking.as_str(), blocking, "{name}");
        assert_eq!(stored.topic_id.as_str(), reply.body["topicId"], "{name}");
        assert_eq!(stored.thread_id.as_str(), reply.body["threadId"], "{name}");
        if name.starts_with("message-thread-session") {
            session_threads.push(stored.thread_id.clone());
        }
        if name == "action-deferrable.json" {
            assert!(stored.actions.iter().any(|action| action.id == "done"));
            assert!(stored.actions.iter().any(|action| action.id == "defer"));
            assert!(!stored.steps.is_empty());
        }
    }
    // Two sessions, one thread (ADR 0009).
    assert_eq!(session_threads.len(), 2);
    assert_eq!(session_threads[0], session_threads[1]);
    server.stop().await;
}

#[tokio::test]
async fn one_message_per_type_and_blocking_level() {
    let server = Server::start().await;
    let rows = [
        ("decision_required", "blocking", true),
        ("decision_required", "deferrable", true),
        ("decision_required", "non_blocking", true),
        ("action_required", "blocking", true),
        ("action_required", "deferrable", true),
        ("action_required", "non_blocking", true),
        ("task_completed", "non_blocking", false),
        ("generic", "non_blocking", false),
    ];
    for (index, (message_type, blocking, should_wait)) in rows.into_iter().enumerate() {
        let reply = server
            .post_message(json!({
                "type": message_type, "blocking": blocking, "awaitsResponse": true,
                "title": "Pick the release name",
                "conversation": { "id": format!("row-{index}") },
                "steps": if message_type == "action_required" { json!([{ "text": "Open the release page" }]) } else { Value::Null },
            }).as_object().map(|fields| {
                Value::Object(fields.iter().filter(|(_, value)| !value.is_null()).map(|(key, value)| (key.clone(), value.clone())).collect())
            }).unwrap())
            .await;
        assert_eq!(reply.status, 201, "{message_type} {blocking}: {reply:?}");
        assert_eq!(
            reply.body["shouldWait"], should_wait,
            "{message_type} {blocking}"
        );
        assert_eq!(reply.body["blocking"], blocking);
    }
    server.stop().await;
}

#[tokio::test]
async fn long_poll_pending_then_answered_mid_wait() {
    let server = Server::start().await;
    let id = id_of(&server.post_message(blocking_prompt()).await);
    let pending = server.poll(&id, 1).await;
    assert_eq!(pending, json!({ "messageId": id, "status": "pending" }));

    let poll = spawn_poll(&server, &id, 30);
    support::wait_until_polling(&server, &id).await;
    answer(&server, &id, "allow").await.unwrap();
    let answered = poll.await.unwrap();
    assert_eq!(answered["status"], "answered");
    assert_eq!(answered["actionId"], "allow");
    assert_eq!(answered["actionKind"], "approve");
    assert_eq!(answered["answeredVia"], "mac");
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_concurrent_answers_one_accepted_one_409() {
    let server = Server::start().await;
    let id = id_of(&server.post_message(blocking_prompt()).await);
    let poll = spawn_poll(&server, &id, 30);
    support::wait_until_polling(&server, &id).await;
    let (first, second) = tokio::join!(answer(&server, &id, "allow"), answer(&server, &id, "deny"));
    let outcomes = [first, second];
    assert_eq!(outcomes.iter().filter(|outcome| outcome.is_ok()).count(), 1);
    let refused = outcomes.into_iter().find_map(Result::err).unwrap();
    let status = ApiError::from(refused).status();
    assert_eq!(status.as_u16(), 409);
    assert_eq!(poll.await.unwrap()["status"], "answered");
    server.stop().await;
}

#[tokio::test]
async fn delete_and_the_conversation_rescind_wake_waiters_with_rescinded() {
    let server = Server::start().await;
    let id = id_of(&server.post_message(blocking_prompt()).await);
    let poll = spawn_poll(&server, &id, 30);
    support::wait_until_polling(&server, &id).await;
    let deleted = server
        .send_for("DELETE", &format!("/v1/messages/{id}"), None)
        .await;
    assert_eq!(deleted.status, 200);
    assert_eq!(deleted.body["status"], "rescinded");
    let woken = poll.await.unwrap();
    assert_eq!(woken["status"], "rescinded");
    assert_eq!(woken["rescindReason"], "resolved_elsewhere");
    let again = server
        .send_for(
            "DELETE",
            &format!("/v1/messages/{id}"),
            Some(json!({ "reason": "obsolete" })),
        )
        .await;
    assert_eq!(again.status, 409);
    assert_eq!(again.body["code"], "already_closed");

    let mut keyed = blocking_prompt();
    keyed["correlationKey"] = json!("ask-relay-name");
    let id = id_of(&server.post_message(keyed).await);
    let poll = spawn_poll(&server, &id, 30);
    support::wait_until_polling(&server, &id).await;
    let rescinded = server
        .send(
            "POST",
            "/v1/conversations/claude-code/conversation-1/rescind",
            Some(example("rescind-request.json")),
        )
        .await;
    assert_eq!(rescinded.status, 200);
    assert_eq!(rescinded.body["rescinded"], json!([id]));
    let woken = poll.await.unwrap();
    assert_eq!(woken["status"], "rescinded");
    assert_eq!(woken["rescindReason"], "obsolete");
    server.stop().await;
}

#[tokio::test]
async fn another_agent_cannot_delete_a_message_and_the_agent_header_is_required() {
    let server = Server::start().await;
    let id = id_of(&server.post_message(blocking_prompt()).await);
    let anonymous = server
        .send("DELETE", &format!("/v1/messages/{id}"), None)
        .await;
    assert_eq!(anonymous.status, 400);
    assert_eq!(anonymous.body["code"], "agent_required");
    let foreign = server
        .send_as(
            "DELETE",
            &format!("/v1/messages/{id}"),
            None,
            &[("Gentbox-Agent", "codex")],
        )
        .await;
    assert_eq!(foreign.status, 403);
    assert_eq!(foreign.body["code"], "not_owner");
    let other_conversation = server
        .send_as(
            "DELETE",
            &format!("/v1/messages/{id}"),
            None,
            &[
                ("Gentbox-Agent", "claude-code"),
                ("Gentbox-Conversation", "conversation-9"),
            ],
        )
        .await;
    assert_eq!(other_conversation.status, 403);
    let moved = server
        .send_as(
            "PUT",
            &format!("/v1/messages/{id}/thread"),
            Some(json!({ "thread": { "key": "elsewhere" } })),
            &[("Gentbox-Agent", "codex")],
        )
        .await;
    assert_eq!(moved.status, 403);
    assert_eq!(server.poll(&id, 0).await["status"], "pending");
    let keys_on_delete = server
        .send_for(
            "DELETE",
            &format!("/v1/messages/{id}"),
            Some(json!({ "reason": "obsolete", "correlationKeys": ["x"] })),
        )
        .await;
    assert_eq!(keys_on_delete.status, 422);
    server.stop().await;
}

#[tokio::test]
async fn defer_wakes_the_waiter_with_deferred_then_pending() {
    let server = Server::start().await;
    let id = id_of(
        &server
            .post_message(json!({
                "type": "decision_required", "blocking": "deferrable", "awaitsResponse": true,
                "title": "Which channel should the relay use?",
                "actions": [{ "id": "apns", "label": "APNs", "kind": "choice", "value": "apns" }]
            }))
            .await,
    );
    let poll = spawn_poll(&server, &id, 30);
    support::wait_until_polling(&server, &id).await;
    answer(&server, &id, "defer").await.unwrap();
    assert_eq!(poll.await.unwrap()["status"], "deferred");
    assert_eq!(server.poll(&id, 0).await["status"], "pending");
    server.stop().await;
}

#[tokio::test]
async fn dismissed_and_expired_wake_the_waiter() {
    let server = Server::start().await;
    let id = id_of(
        &server
            .post_message(json!({ "type": "decision_required", "title": "Rename the crate?", "awaitsResponse": true }))
            .await,
    );
    let poll = spawn_poll(&server, &id, 30);
    support::wait_until_polling(&server, &id).await;
    server.handle.dismiss(MessageId::new(&id)).await.unwrap();
    assert_eq!(poll.await.unwrap()["status"], "dismissed");

    let id = id_of(
        &server
            .post_message(json!({
                "type": "decision_required", "blocking": "blocking", "title": "Approve the deploy?",
                "awaitsResponse": true, "conversation": { "id": "conversation-2" },
                "expiresAt": "2026-10-24T12:01:00Z"
            }))
            .await,
    );
    let poll = spawn_poll(&server, &id, 30);
    support::wait_until_polling(&server, &id).await;
    server.clock.advance(time::Duration::minutes(1));
    server.handle.run_due_timers().await.unwrap();
    assert_eq!(poll.await.unwrap()["status"], "expired");
    server.stop().await;
}

#[tokio::test]
async fn a_waiterless_done_reaches_a_later_poll_and_a_swept_id_is_gone() {
    let server = Server::start().await;
    let id = id_of(
        &server
            .post_message(json!({
                "type": "action_required", "blocking": "blocking", "event": "claude-code.stop_failure",
                "title": "The turn ended on an API error",
                "steps": [{ "text": "Open the session" }, { "text": "Ask Claude to try again" }]
            }))
            .await,
    );
    answer(&server, &id, "done").await.unwrap();
    let later = server.poll(&id, 0).await;
    assert_eq!(later["status"], "answered");
    assert_eq!(later["actionKind"], "done");

    server.clock.advance(time::Duration::days(31));
    server.handle.run_due_timers().await.unwrap();
    let gone = server.poll(&id, 5).await;
    assert_eq!(gone, json!({ "messageId": id, "status": "gone" }));
    let unknown = server
        .send(
            "GET",
            "/v1/messages/01JBM3Z8Q7Y4K2N6P0R5T9VWXA/response?wait=0",
            None,
        )
        .await;
    assert_eq!(unknown.status, 404);
    let malformed = server.send("GET", "/v1/messages/nope/response", None).await;
    assert_eq!(malformed.status, 404);
    server.stop().await;
}

#[tokio::test]
async fn shutting_down_answers_a_waiting_poll_pending() {
    let server = Server::start().await;
    let id = id_of(&server.post_message(blocking_prompt()).await);
    let poll = spawn_poll(&server, &id, 55);
    support::wait_until_polling(&server, &id).await;
    let started = std::time::Instant::now();
    server.stop().await;
    assert_eq!(poll.await.unwrap()["status"], "pending");
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[tokio::test]
async fn a_command_title_is_rewritten_with_a_warning_and_the_original_leads_the_body() {
    let server = Server::start().await;
    let reply = server
        .post_message(json!({
            "type": "task_completed",
            "title": "cargo test -p gentbox-ingest -- --nocapture",
            "body": "All 42 tests pass."
        }))
        .await;
    assert_eq!(reply.status, 201);
    assert_eq!(
        reply.body["warnings"][0]["code"],
        "title_looks_like_command"
    );
    let stored = server
        .handle
        .message(MessageId::new(id_of(&reply)))
        .await
        .unwrap();
    assert_eq!(stored.title, "Claude Code finished a task in gentbox");
    let body = stored.body.unwrap();
    assert!(body.starts_with("**Original title**"), "{body}");
    assert!(body.contains("cargo test -p gentbox-ingest -- --nocapture"));
    server.stop().await;
}

#[tokio::test]
async fn strict_sources_get_422_for_command_titles_and_missing_plans() {
    use gentbox_core::action_plan::ActionPlanMode;
    use gentbox_core::prepare::IngestSettings;
    use gentbox_core::title_guard::GuardMode;
    let server = Server::start_with(
        |config| {
            config.sources.insert(
                "my-adapter".to_owned(),
                IngestSettings {
                    title_guard: GuardMode::Reject,
                    action_plan: ActionPlanMode::Reject,
                },
            );
        },
        gentbox_ingest::Limits::default(),
    )
    .await;
    let strict = json!({ "source": { "agent": "my-adapter" } });
    let mut title = strict.clone();
    title["title"] = json!("npm publish --tag next");
    let refused = server.post_message(title).await;
    assert_eq!(refused.status, 422);
    assert_eq!(refused.body["code"], "title_looks_like_command");

    let mut plan = strict;
    plan["type"] = json!("action_required");
    plan["title"] = json!("Sign in to npm");
    let refused = server.post_message(plan).await;
    assert_eq!(refused.status, 422);
    assert_eq!(refused.body["code"], "steps_missing");
    server.stop().await;
}

#[tokio::test]
async fn a_missing_plan_gets_the_fallback_step_and_a_warning() {
    let server = Server::start().await;
    let reply = server
        .post_message(json!({
            "type": "action_required", "title": "Review the pull request",
            "conversation": { "id": "c", "focus": { "url": "claude-cli://open?cwd=/x" } }
        }))
        .await;
    assert_eq!(reply.status, 201);
    let codes: Vec<&str> = reply.body["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|warning| warning["code"].as_str().unwrap())
        .collect();
    assert!(codes.contains(&"steps_missing"), "{codes:?}");
    let stored = server
        .handle
        .message(MessageId::new(id_of(&reply)))
        .await
        .unwrap();
    assert_eq!(stored.steps.len(), 1);
    assert_eq!(
        stored.steps[0].text,
        "Open the conversation and follow the agent's instructions"
    );
    assert!(stored.actions.iter().any(|action| action.id == "done"));
    server.stop().await;
}

#[tokio::test]
async fn ticks_persist_and_never_reach_the_long_poll() {
    let server = Server::start().await;
    let id = id_of(
        &server
            .post_message(json!({
                "type": "action_required", "blocking": "blocking", "awaitsResponse": true,
                "title": "Sign in to npm",
                "steps": [{ "text": "Open a terminal" }, { "text": "Sign in to npm" }]
            }))
            .await,
    );
    let poll = spawn_poll(&server, &id, 1);
    support::wait_until_polling(&server, &id).await;
    server
        .handle
        .tick_step(MessageId::new(&id), 0, true)
        .await
        .unwrap();
    assert_eq!(poll.await.unwrap()["status"], "pending");
    let stored = server.handle.message(MessageId::new(&id)).await.unwrap();
    assert_eq!(stored.steps_completed, [0]);
    server.stop().await;
}

#[tokio::test]
async fn a_flagged_conversation_title_is_ignored_with_a_warning() {
    let server = Server::start().await;
    let reply = server
        .post_message(
            json!({ "conversation": { "id": "c", "title": "~/Developer/gentbox: git status" } }),
        )
        .await;
    assert_eq!(reply.status, 201);
    assert!(
        reply.body["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| warning["code"] == "conversation_title_ignored")
    );
    let stored = server
        .handle
        .message(MessageId::new(id_of(&reply)))
        .await
        .unwrap();
    assert_eq!(stored.conversation.title, None);

    let renamed = server
        .send(
            "PUT",
            "/v1/conversations/claude-code/c",
            Some(json!({ "id": "c", "title": "Ship the ingest server" })),
        )
        .await;
    assert_eq!(renamed.status, 200);
    let thread = server.handle.thread(stored.thread_id).await.unwrap();
    assert_eq!(
        thread.title, "Ship the ingest server",
        "the implicit thread follows"
    );
    let mismatched = server
        .send(
            "PUT",
            "/v1/conversations/claude-code/c",
            Some(json!({ "id": "d" })),
        )
        .await;
    assert_eq!(mismatched.status, 422);
    server.stop().await;
}

#[tokio::test]
async fn progress_rescinds_stale_blocking_items_and_an_idempotent_retry_is_200() {
    let server = Server::start().await;
    let stale = server
        .post_message(json!({ "type": "decision_required", "blocking": "blocking", "title": "Allow the build?" }))
        .await;
    let progress = server
        .send(
            "POST",
            "/v1/conversations/claude-code/conversation-1/progress",
            None,
        )
        .await;
    assert_eq!(progress.status, 200);
    assert_eq!(progress.body["rescinded"], json!([id_of(&stale)]));
    let state = server
        .handle
        .message(MessageId::new(id_of(&stale)))
        .await
        .unwrap()
        .lifecycle
        .state;
    assert_eq!(state, LifecycleState::Rescinded);

    let retry =
        json!({ "idempotencyKey": "k-1", "type": "task_completed", "title": "Phase 3 is done" });
    let first = server.post_message(retry.clone()).await;
    let second = server.post_message(retry).await;
    assert_eq!((first.status, second.status), (201, 200));
    assert_eq!(first.body["id"], second.body["id"]);
    server.stop().await;
}

#[tokio::test]
async fn presence_focus_is_accepted() {
    let server = Server::start().await;
    let reply = server
        .send(
            "POST",
            "/v1/presence/focus",
            Some(json!({ "focused": true })),
        )
        .await;
    assert_eq!(reply.status, 204);
    let wrong = server
        .send("POST", "/v1/presence/focus", Some(json!({ "focus": true })))
        .await;
    assert_eq!(wrong.status, 422);
    server.stop().await;
}
