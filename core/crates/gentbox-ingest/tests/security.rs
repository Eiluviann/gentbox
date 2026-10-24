//! Transport and security (architecture §8, §12): the token, the `Host`
//! check on TCP only, the `hello` proof, size, schema and rate limits, no
//! answer or list route, and the discovery files.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::os::unix::fs::PermissionsExt;

use gentbox_ingest::{Limits, hello_proof};
use serde_json::json;
use support::{Server, blocking_prompt, message, reply, socket_request};

fn mode(path: &std::path::Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[tokio::test]
async fn a_missing_or_wrong_token_is_401() {
    let server = Server::start().await;
    let without = reply(
        server
            .client
            .post(server.url("/v1/messages"))
            .json(&message(json!({})))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(without.status, 401);
    assert_eq!(without.body["code"], "unauthorized");
    assert_eq!(without.headers["www-authenticate"], "Bearer");

    let wrong = reply(
        server
            .client
            .post(server.url("/v1/messages"))
            .bearer_auth(format!("{}x", server.token))
            .json(&message(json!({})))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(wrong.status, 401);
    let basic = reply(
        server
            .client
            .get(server.url("/v1/topics"))
            .header("authorization", format!("Basic {}", server.token))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(basic.status, 401);
    server.stop().await;
}

#[tokio::test]
async fn a_foreign_host_over_tcp_is_400_and_the_socket_is_not_checked() {
    let server = Server::start().await;
    for host in [
        "evil.example".to_owned(),
        format!("evil.example:{}", server.port),
        "127.0.0.1:1".to_owned(),
    ] {
        let refused = reply(
            server
                .client
                .get(server.url("/v1/topics"))
                .bearer_auth(&server.token)
                .header("host", &host)
                .send()
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(refused.status, 400, "{host}");
        assert_eq!(refused.body["code"], "bad_host");
    }
    let localhost = reply(
        server
            .client
            .get(server.url("/v1/topics"))
            .bearer_auth(&server.token)
            .header("host", format!("localhost:{}", server.port))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(localhost.status, 200);

    let bearer = format!("Bearer {}", server.token);
    let socket = server.api_dir().join("hub.sock");
    let (status, _) = socket_request(
        &socket,
        "GET",
        "/v1/topics",
        &[("host", "evil.example"), ("authorization", &bearer)],
        None,
    )
    .await;
    assert_eq!(status, 200);
    let (status, _) =
        socket_request(&socket, "GET", "/v1/topics", &[("host", "localhost")], None).await;
    assert_eq!(status, 401, "the socket still needs the token");
    server.stop().await;
}

#[tokio::test]
async fn hello_proves_the_token_and_never_needs_it() {
    let server = Server::start().await;
    let nonce = "q8F3kz0WmPz1vN7yT2cR5aLx";
    let answer = reply(
        server
            .client
            .post(server.url("/v1/hello"))
            .json(&json!({ "nonce": nonce }))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(answer.status, 200);
    assert_eq!(answer.body["proof"], hello_proof(&server.token, nonce));
    assert_ne!(answer.body["proof"], hello_proof("another-token", nonce));
    assert_eq!(answer.body["hub"], "gentbox");
    assert_eq!(answer.body["schemaVersions"], json!([1]));
    assert!(
        !answer.body.to_string().contains(&server.token),
        "hello never reveals the token"
    );

    let short = reply(
        server
            .client
            .post(server.url("/v1/hello"))
            .json(&json!({ "nonce": "short" }))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(short.status, 422);
    assert_eq!(short.body["pointer"], "/nonce");
    server.stop().await;
}

#[tokio::test]
async fn a_body_over_128_kib_is_413() {
    let server = Server::start().await;
    let big = message(json!({ "body": "x".repeat(130 * 1024) }));
    let refused = server.send("POST", "/v1/messages", Some(big)).await;
    assert_eq!(refused.status, 413);
    assert_eq!(refused.body["code"], "too_large");
    server.stop().await;
}

#[tokio::test]
async fn an_invalid_message_is_422_with_the_json_pointer() {
    let server = Server::start().await;
    let long_title = server
        .post_message(json!({ "title": "t".repeat(121) }))
        .await;
    assert_eq!(long_title.status, 422);
    assert_eq!(long_title.body["code"], "schema_invalid");
    assert_eq!(long_title.body["pointer"], "/title");

    let typo = server.post_message(json!({ "urgnet": true })).await;
    assert_eq!(typo.status, 422);
    let declared_generic_blocking = server
        .post_message(json!({ "type": "generic", "blocking": "blocking" }))
        .await;
    assert_eq!(declared_generic_blocking.status, 422);
    assert_eq!(declared_generic_blocking.body["pointer"], "/blocking");
    let bad_step = server
        .post_message(json!({ "type": "action_required", "steps": [{ "text": "Open it", "link": { "url": "javascript:alert(1)" } }] }))
        .await;
    assert_eq!(bad_step.status, 422);
    server.stop().await;
}

#[tokio::test]
async fn only_json_bodies_are_accepted() {
    let server = Server::start().await;
    let text = reply(
        server
            .client
            .post(server.url("/v1/messages"))
            .bearer_auth(&server.token)
            .header("content-type", "text/plain")
            .body(message(json!({})).to_string())
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(text.status, 415);
    let broken = reply(
        server
            .client
            .post(server.url("/v1/messages"))
            .bearer_auth(&server.token)
            .header("content-type", "application/json")
            .body("{ not json")
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(broken.status, 400);
    assert_eq!(broken.body["code"], "invalid_json");
    server.stop().await;
}

#[tokio::test]
async fn over_the_rate_limit_is_429_with_retry_after() {
    let server = Server::start().await;
    for index in 0..10 {
        let accepted = server
            .post_message(
                json!({ "title": format!("Step {index} is done"), "type": "task_completed" }),
            )
            .await;
        assert!(
            accepted.status == 201 || accepted.status == 200,
            "{accepted:?}"
        );
    }
    let limited = server
        .post_message(json!({ "title": "Step ten is done", "type": "task_completed" }))
        .await;
    assert_eq!(limited.status, 429);
    assert_eq!(limited.body["code"], "rate_limited");
    let retry_after: u64 = limited.headers["retry-after"]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!(retry_after >= 1);
    // Another conversation of the same agent still gets through.
    let other = server
        .post_message(json!({ "conversation": { "id": "conversation-2" } }))
        .await;
    assert_eq!(other.status, 201);
    server.stop().await;
}

#[tokio::test]
async fn the_hello_route_is_rate_limited_too() {
    let limits = Limits {
        hello_burst: 2.0,
        hello_refill_per_second: 0.01,
        ..Limits::default()
    };
    let server = Server::start_with(|_| {}, limits).await;
    let nonce = json!({ "nonce": "q8F3kz0WmPz1vN7yT2cR5aLx" });
    let mut statuses = Vec::new();
    for _ in 0..3 {
        let answer = server
            .client
            .post(server.url("/v1/hello"))
            .json(&nonce)
            .send()
            .await
            .unwrap();
        statuses.push(answer.status().as_u16());
    }
    assert_eq!(statuses, [200, 200, 429]);
    server.stop().await;
}

#[tokio::test]
async fn no_route_answers_or_lists_messages_with_the_sender_token() {
    let server = Server::start().await;
    let posted = server.post_message(blocking_prompt()).await;
    let id = support::id_of(&posted);
    let attempts = [
        ("GET", "/v1/messages".to_owned()),
        ("GET", format!("/v1/messages/{id}")),
        ("POST", format!("/v1/messages/{id}/answer")),
        ("POST", format!("/v1/messages/{id}/actions/allow")),
        ("POST", format!("/v1/messages/{id}/response")),
        ("PUT", format!("/v1/messages/{id}/response")),
        ("GET", "/v1/inbox".to_owned()),
        (
            "GET",
            format!(
                "/v1/topics/{}/messages",
                posted.body["topicId"].as_str().unwrap()
            ),
        ),
    ];
    for (method, path) in attempts {
        let refused = server
            .send(method, &path, Some(json!({ "actionId": "allow" })))
            .await;
        assert!(
            refused.status == 404 || refused.status == 405,
            "{method} {path}: {refused:?}"
        );
    }
    let still = server.poll(&id, 0).await;
    assert_eq!(still["status"], "pending");

    // Listings return groups only, never messages.
    let topics = server.send("GET", "/v1/topics", None).await;
    let threads = server.send("GET", "/v1/threads", None).await;
    for listing in [topics.body.to_string(), threads.body.to_string()] {
        assert!(!listing.contains("Allow Bash"), "{listing}");
        assert!(!listing.contains(&id), "{listing}");
    }
    server.stop().await;
}

#[tokio::test]
async fn the_discovery_files_are_private_and_port_goes_on_clean_shutdown() {
    let server = Server::start().await;
    let api = server.api_dir();
    assert_eq!(mode(&api), 0o700);
    assert_eq!(mode(&api.join("token")), 0o600);
    assert_eq!(mode(&api.join("port")), 0o600);
    assert_eq!(mode(&api.join("hub.sock")), 0o600);
    let token = std::fs::read_to_string(api.join("token")).unwrap();
    assert_eq!(token.len(), 43, "256 bits as base64url");
    assert_eq!(
        std::fs::read_to_string(api.join("port")).unwrap(),
        server.port.to_string()
    );
    let directory = server.directory.path().to_path_buf();
    // Keep the temp dir alive across the shutdown.
    let Server {
        hub,
        ingest,
        directory: keep,
        ..
    } = server;
    ingest.unwrap().shutdown().await;
    hub.unwrap().shutdown().await;
    assert!(!directory.join("API/port").exists());
    assert!(!directory.join("API/hub.sock").exists());
    assert!(directory.join("API/token").exists(), "the token stays");
    drop(keep);
}

#[tokio::test]
async fn the_token_survives_a_restart_and_a_loose_folder_is_tightened() {
    let directory = tempfile::tempdir().unwrap();
    let first = gentbox_ingest::ApiDir::prepare(directory.path()).unwrap();
    std::fs::set_permissions(first.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::set_permissions(
        first.path().join("token"),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    std::fs::write(first.path().join("port"), "1").unwrap();
    let second = gentbox_ingest::ApiDir::prepare(directory.path()).unwrap();
    assert_eq!(first.token(), second.token());
    assert_eq!(mode(second.path()), 0o700);
    assert_eq!(mode(&second.path().join("token")), 0o600);
    assert!(
        !second.path().join("port").exists(),
        "a stale port is removed"
    );
}

#[tokio::test]
async fn the_socket_serves_the_whole_api() {
    let server = Server::start().await;
    let socket = server.api_dir().join("hub.sock");
    let bearer = format!("Bearer {}", server.token);
    let headers = [("host", "localhost"), ("authorization", bearer.as_str())];
    let (status, body) = socket_request(
        &socket,
        "POST",
        "/v1/messages",
        &headers,
        Some(message(blocking_prompt())),
    )
    .await;
    assert_eq!(status, 201);
    let id = body["id"].as_str().unwrap();
    let (status, body) = socket_request(
        &socket,
        "GET",
        &format!("/v1/messages/{id}/response?wait=0"),
        &headers,
        None,
    )
    .await;
    assert_eq!((status, body["status"].as_str()), (200, Some("pending")));
    server.stop().await;
}

#[tokio::test]
async fn malformed_ids_are_422_and_never_fill_the_rate_limiter() {
    let server = Server::start().await;
    for index in 0..60 {
        let refused = server
            .post_message(json!({ "source": { "agent": format!("Bad Agent {index}") } }))
            .await;
        assert_eq!(refused.status, 422, "{index}: {refused:?}");
    }
    let accepted = server.post_message(json!({})).await;
    assert_eq!(accepted.status, 201);
    server.stop().await;
}

#[tokio::test]
async fn a_state_folder_too_deep_for_a_socket_still_serves_tcp() {
    let directory = tempfile::tempdir().unwrap();
    let deep = directory.path().join("d".repeat(120));
    let hub = gentbox_hub::Hub::start(
        gentbox_hub::HubConfig::new(&deep),
        std::sync::Arc::new(gentbox_hub::NoDelegate),
        None,
    )
    .unwrap();
    let ingest = gentbox_ingest::IngestServer::start(hub.handle(), &deep, Limits::default())
        .await
        .unwrap();
    assert!(!deep.join("API/hub.sock").exists());
    let answer = reqwest::Client::new()
        .get(format!("http://127.0.0.1:{}/v1/topics", ingest.port()))
        .bearer_auth(ingest.api_dir().token())
        .send()
        .await
        .unwrap();
    assert_eq!(answer.status().as_u16(), 200);
    ingest.shutdown().await;
    hub.shutdown().await;
}

#[tokio::test]
async fn a_client_stalled_mid_request_cannot_hold_the_shutdown() {
    use tokio::io::AsyncWriteExt;
    let server = Server::start().await;
    let mut stalled = tokio::net::TcpStream::connect(("127.0.0.1", server.port))
        .await
        .unwrap();
    let head = format!(
        "POST /v1/messages HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: 1000\r\n\r\n{{",
        server.port, server.token
    );
    stalled.write_all(head.as_bytes()).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let api = server.api_dir();
    let started = std::time::Instant::now();
    server.stop().await;
    assert!(started.elapsed() < std::time::Duration::from_secs(8));
    assert!(!api.join("port").exists());
    drop(stalled);
}

#[tokio::test]
async fn every_error_has_the_json_shape_and_cache_busters_are_ignored() {
    let server = Server::start().await;
    let unknown_query = server.send("GET", "/v1/topics?foo=1", None).await;
    assert_eq!(unknown_query.status, 400);
    assert_eq!(unknown_query.body["code"], "invalid_query");
    let wrong_method = server.send("GET", "/v1/messages", None).await;
    assert_eq!(wrong_method.status, 405);
    assert_eq!(wrong_method.body["code"], "method_not_allowed");
    let id = support::id_of(&server.post_message(blocking_prompt()).await);
    let busted = server
        .send(
            "GET",
            &format!("/v1/messages/{id}/response?wait=0&_=1712"),
            None,
        )
        .await;
    assert_eq!(busted.status, 200, "{busted:?}");
    assert_eq!(busted.body["status"], "pending");
    let bad_wait = server
        .send(
            "GET",
            &format!("/v1/messages/{id}/response?wait=soon"),
            None,
        )
        .await;
    assert_eq!(bad_wait.status, 400);
    assert_eq!(bad_wait.body["code"], "invalid_wait");
    server.stop().await;
}

#[tokio::test]
async fn debug_output_never_shows_the_token() {
    let server = Server::start().await;
    let ingest = server.ingest.as_ref().unwrap();
    for printed in [format!("{ingest:?}"), format!("{:?}", ingest.api_dir())] {
        assert!(!printed.contains(&server.token), "{printed}");
        assert!(printed.contains("redacted"), "{printed}");
    }
    server.stop().await;
}

#[tokio::test]
async fn hello_checks_its_rate_limit_before_reading_and_takes_small_bodies_only() {
    let limits = Limits {
        hello_burst: 1.0,
        hello_refill_per_second: 0.01,
        ..Limits::default()
    };
    let server = Server::start_with(|_| {}, limits).await;
    let big = json!({ "nonce": "q8F3kz0WmPz1vN7yT2cR5aLx", "padding": "x".repeat(4096) });
    let too_big = reply(
        server
            .client
            .post(server.url("/v1/hello"))
            .json(&big)
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(too_big.status, 413, "hello bodies are capped small");
    // The bucket is empty now: a huge body is refused unread.
    let huge = json!({ "nonce": "x".repeat(100 * 1024) });
    let limited = reply(
        server
            .client
            .post(server.url("/v1/hello"))
            .json(&huge)
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(limited.status, 429);
    server.stop().await;
}
