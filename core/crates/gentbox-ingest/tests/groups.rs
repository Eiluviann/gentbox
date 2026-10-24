//! Topic and thread routes (ADRs 0008, 0009, 0010) through the real router.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use serde_json::{Value, json};
use support::{Server, blocking_prompt, example, id_of};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_upserts_of_one_key_create_one_topic_and_one_thread() {
    let server = Server::start().await;
    let post = |path: &'static str, body: Value| {
        let client = server.client.clone();
        let url = server.url(path);
        let token = server.token.clone();
        tokio::spawn(async move {
            let response = client
                .post(url)
                .bearer_auth(token)
                .header("Gentbox-Agent", "claude-code")
                .json(&body)
                .send()
                .await
                .unwrap();
            let status = response.status().as_u16();
            let body: Value = response.json().await.unwrap();
            (status, body["id"].as_str().unwrap().to_owned())
        })
    };
    let topics: Vec<_> = (0..12)
        .map(|_| post("/v1/topics", example("topic-create.json")))
        .collect();
    let mut statuses = Vec::new();
    let mut ids = std::collections::BTreeSet::new();
    for topic in topics {
        let (status, id) = topic.await.unwrap();
        statuses.push(status);
        ids.insert(id);
    }
    assert_eq!(statuses.iter().filter(|status| **status == 201).count(), 1);
    assert_eq!(statuses.iter().filter(|status| **status == 200).count(), 11);
    assert_eq!(ids.len(), 1);

    let threads: Vec<_> = (0..12)
        .map(|_| post("/v1/threads", example("thread-conversation-takeover.json")))
        .collect();
    let mut created = 0;
    let mut ids = std::collections::BTreeSet::new();
    for thread in threads {
        let (status, id) = thread.await.unwrap();
        created += usize::from(status == 201);
        ids.insert(id);
    }
    assert_eq!((created, ids.len()), (1, 1));
    server.stop().await;
}

#[tokio::test]
async fn creates_need_the_agent_and_record_it() {
    let server = Server::start().await;
    let anonymous = server
        .send("POST", "/v1/topics", Some(example("topic-create.json")))
        .await;
    assert_eq!(anonymous.status, 400);
    let created = server
        .send_for("POST", "/v1/topics", Some(example("topic-create.json")))
        .await;
    assert_eq!(created.status, 201);
    assert_eq!(created.body["createdBy"]["agent"], "claude-code");
    assert_eq!(created.body["key"], "release-1.4");
    assert_eq!(created.body["openMessages"], 0);
    let fetched = server
        .send("GET", &format!("/v1/topics/{}", id_of(&created)), None)
        .await;
    assert_eq!(fetched.body, created.body);
    server.stop().await;
}

#[tokio::test]
async fn closing_is_refused_while_messages_are_open() {
    let server = Server::start().await;
    let mut prompt = blocking_prompt();
    prompt["topic"] = json!({ "key": "release-1.4" });
    prompt["thread"] = json!({ "key": "phase-3" });
    let posted = server.post_message(prompt.clone()).await;
    let id = id_of(&posted);
    let topic = posted.body["topicId"].as_str().unwrap().to_owned();
    let thread = posted.body["threadId"].as_str().unwrap().to_owned();

    for path in [
        format!("/v1/topics/{topic}/close"),
        format!("/v1/threads/{thread}/close"),
    ] {
        let refused = server.send_for("POST", &path, None).await;
        assert_eq!(refused.status, 409, "{path}");
        assert_eq!(refused.body["code"], "has_open_messages");
        assert_eq!(refused.body["openMessages"], 1);
    }
    assert_eq!(server.poll(&id, 0).await["status"], "pending");

    // Withdrawing its own open messages lets the agent close; the waiter
    // hears `rescinded`.
    let poll = {
        let client = server.client.clone();
        let url = server.url(&format!("/v1/messages/{id}/response?wait=30"));
        let token = server.token.clone();
        tokio::spawn(async move {
            client
                .get(url)
                .bearer_auth(token)
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap()
        })
    };
    support::wait_until_polling(&server, &id).await;
    let closed = server
        .send_for(
            "POST",
            &format!("/v1/threads/{thread}/close"),
            Some(example("close-request.json")),
        )
        .await;
    assert_eq!(closed.status, 200);
    assert_eq!(closed.body["status"], "closed");
    let woken = poll.await.unwrap();
    assert_eq!(woken["status"], "rescinded");
    assert_eq!(woken["rescindReason"], "obsolete");

    // A message to the closed thread reopens it and its topic.
    let mut again = prompt.clone();
    again["thread"] = json!({ "id": thread });
    again["conversation"] = json!({ "id": "conversation-2" });
    let reopening = server.post_message(again).await;
    assert_eq!(reopening.status, 201);
    for path in [
        format!("/v1/topics/{topic}"),
        format!("/v1/threads/{thread}"),
    ] {
        assert_eq!(
            server.send("GET", &path, None).await.body["status"],
            "open",
            "{path}"
        );
    }
    server.stop().await;
}

#[tokio::test]
async fn the_general_topic_answers_409_to_close() {
    let server = Server::start().await;
    let posted = server
        .post_message(json!({ "type": "task_completed", "title": "Docs build" }))
        .await;
    let topic = posted.body["topicId"].as_str().unwrap();
    let refused = server
        .send_for("POST", &format!("/v1/topics/{topic}/close"), None)
        .await;
    assert_eq!(refused.status, 409);
    assert_eq!(refused.body["code"], "general_topic");
    server.stop().await;
}

#[tokio::test]
async fn reopen_and_rescind_routes() {
    let server = Server::start().await;
    let mut prompt = blocking_prompt();
    prompt["topic"] = json!({ "key": "release-1.4" });
    let posted = server.post_message(prompt).await;
    let topic = posted.body["topicId"].as_str().unwrap().to_owned();
    let rescinded = server
        .send_for(
            "POST",
            &format!("/v1/topics/{topic}/rescind"),
            Some(json!({ "reason": "obsolete" })),
        )
        .await;
    assert_eq!(rescinded.status, 200);
    assert_eq!(rescinded.body["rescinded"], json!([id_of(&posted)]));
    let closed = server
        .send_for(
            "POST",
            &format!("/v1/topics/{topic}/close"),
            Some(json!({})),
        )
        .await;
    assert_eq!(closed.body["status"], "closed");
    let reopened = server
        .send("POST", &format!("/v1/topics/{topic}/reopen"), None)
        .await;
    assert_eq!(reopened.body["status"], "open");
    let listing = server.send("GET", "/v1/topics?status=open", None).await;
    assert_eq!(listing.body["topics"][0]["openMessages"], 0);
    let bad = server.send("GET", "/v1/topics?status=maybe", None).await;
    assert_eq!(bad.status, 400);
    server.stop().await;
}

#[tokio::test]
async fn implicit_groups_answer_409_to_patches() {
    let server = Server::start().await;
    let posted = server.post_message(blocking_prompt()).await;
    let topic = posted.body["topicId"].as_str().unwrap();
    let thread = posted.body["threadId"].as_str().unwrap();
    let general = server
        .send("GET", &format!("/v1/topics/{topic}"), None)
        .await;
    assert_eq!(general.body["implicit"], true);
    assert_eq!(general.body["kind"], "general");
    let patched = server
        .send(
            "PATCH",
            &format!("/v1/topics/{topic}"),
            Some(json!({ "title": "Mine" })),
        )
        .await;
    assert_eq!(patched.status, 409);
    assert_eq!(patched.body["code"], "implicit_group");
    let patched = server
        .send(
            "PATCH",
            &format!("/v1/threads/{thread}"),
            Some(json!({ "title": "Mine" })),
        )
        .await;
    assert_eq!(patched.status, 409);
    server.stop().await;
}

#[tokio::test]
async fn flagged_titles_on_explicit_creates_and_patches_are_422() {
    let server = Server::start().await;
    let flagged = server
        .send_for(
            "POST",
            "/v1/topics",
            Some(json!({ "key": "bad", "title": "git push origin main" })),
        )
        .await;
    assert_eq!(flagged.status, 422);
    assert_eq!(flagged.body["code"], "title_looks_like_command");
    let flagged = server
        .send_for(
            "POST",
            "/v1/threads",
            Some(json!({ "key": "bad", "title": "~/Developer/gentbox/core" })),
        )
        .await;
    assert_eq!(flagged.status, 422);

    let topic = server
        .send_for("POST", "/v1/topics", Some(example("topic-create.json")))
        .await;
    let id = id_of(&topic);
    let flagged = server
        .send(
            "PATCH",
            &format!("/v1/topics/{id}"),
            Some(json!({ "title": "rm -rf build" })),
        )
        .await;
    assert_eq!(flagged.status, 422);
    assert_eq!(flagged.body["code"], "title_looks_like_command");
    let cleared = server
        .send(
            "PATCH",
            &format!("/v1/topics/{id}"),
            Some(json!({ "summary": null, "title": "Release 1.4 and docs" })),
        )
        .await;
    assert_eq!(cleared.status, 200);
    assert_eq!(cleared.body["title"], "Release 1.4 and docs");
    assert!(cleared.body.get("summary").is_none());
    let empty = server
        .send("PATCH", &format!("/v1/topics/{id}"), Some(json!({})))
        .await;
    assert_eq!(empty.status, 422, "minProperties: 1");
    server.stop().await;
}

#[tokio::test]
async fn a_thread_move_into_a_topic_with_its_key_is_409() {
    let server = Server::start().await;
    let first = server
        .post_message(
            json!({ "title": "One", "topic": { "key": "a" }, "thread": { "key": "phase-3" } }),
        )
        .await;
    let second = server
        .post_message(json!({ "title": "Two", "topic": { "key": "b" }, "thread": { "key": "phase-3" }, "conversation": { "id": "c2" } }))
        .await;
    let thread = first.body["threadId"].as_str().unwrap();
    let conflict = server
        .send_for(
            "PUT",
            &format!("/v1/threads/{thread}/topic"),
            Some(json!({ "topic": { "id": second.body["topicId"] } })),
        )
        .await;
    assert_eq!(conflict.status, 409);
    let moved = server
        .send_for(
            "PUT",
            &format!("/v1/threads/{thread}/topic"),
            Some(json!({ "topic": { "key": "c", "title": "Topic C" } })),
        )
        .await;
    assert_eq!(moved.status, 200);
    assert_ne!(moved.body["topicId"], first.body["topicId"]);
    let unknown = server
        .send_for(
            "PUT",
            &format!("/v1/threads/{thread}/topic"),
            Some(json!({ "topic": { "id": "01JBM6R3K9P2W5X8Z1C4V7N0QD" } })),
        )
        .await;
    assert_eq!(unknown.status, 422);
    assert_eq!(unknown.body["code"], "unknown_topic");
    server.stop().await;
}

#[tokio::test]
async fn a_message_moves_to_another_thread_and_back_to_its_default() {
    let server = Server::start().await;
    let posted = server.post_message(blocking_prompt()).await;
    let id = id_of(&posted);
    let moved = server
        .send_for(
            "PUT",
            &format!("/v1/messages/{id}/thread"),
            Some(json!({ "topic": { "key": "release-1.4" }, "thread": { "key": "phase-3" } })),
        )
        .await;
    assert_eq!(moved.status, 200, "{moved:?}");
    assert_ne!(moved.body["threadId"], posted.body["threadId"]);
    let back = server
        .send_for(
            "PUT",
            &format!("/v1/messages/{id}/thread"),
            Some(json!({ "thread": null, "topic": { "id": posted.body["topicId"] } })),
        )
        .await;
    assert_eq!(back.status, 200);
    assert_eq!(back.body["threadId"], posted.body["threadId"]);
    let conflict = server
        .post_message(json!({
            "title": "Three", "conversation": { "id": "c3" },
            "thread": { "id": posted.body["threadId"] }, "topic": { "key": "elsewhere" }
        }))
        .await;
    assert_eq!(conflict.status, 422);
    assert_eq!(conflict.body["code"], "topic_conflict");
    server.stop().await;
}
