//! A hub and its listeners on a temp state folder, with clients for TCP and
//! the Unix socket.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use gentbox_hub::{Hub, HubConfig, HubHandle, ManualClock, NoDelegate};
use gentbox_ingest::{IngestServer, Limits};
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use serde_json::{Value, json};
use tempfile::TempDir;
use time::macros::datetime;

pub struct Server {
    pub hub: Option<Hub>,
    pub handle: HubHandle,
    pub ingest: Option<IngestServer>,
    pub clock: ManualClock,
    pub client: reqwest::Client,
    pub token: String,
    pub port: u16,
    pub directory: TempDir,
}

/// A response: status, headers and JSON body (`Null` when empty).
#[derive(Debug)]
pub struct Reply {
    pub status: u16,
    pub headers: reqwest::header::HeaderMap,
    pub body: Value,
}

impl Server {
    pub async fn start() -> Self {
        Self::start_with(|_| {}, Limits::default()).await
    }

    pub async fn start_with(configure: impl FnOnce(&mut HubConfig), limits: Limits) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let clock = ManualClock::new(datetime!(2026-10-24 12:00:00 UTC));
        let mut config = HubConfig::new(directory.path());
        config.clock = Arc::new(clock.clone());
        configure(&mut config);
        let hub = Hub::start(config, Arc::new(NoDelegate), None).unwrap();
        let handle = hub.handle();
        let ingest = IngestServer::start(handle.clone(), directory.path(), limits)
            .await
            .unwrap();
        let token = ingest.api_dir().token().to_owned();
        let port = ingest.port();
        Self {
            hub: Some(hub),
            handle,
            ingest: Some(ingest),
            clock,
            client: reqwest::Client::new(),
            token,
            port,
            directory,
        }
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    pub fn api_dir(&self) -> PathBuf {
        self.directory.path().join("API")
    }

    /// An authenticated request with an optional JSON body.
    pub async fn send(&self, method: &str, path: &str, body: Option<Value>) -> Reply {
        self.send_as(method, path, body, &[]).await
    }

    /// As [`Server::send`], with extra headers.
    pub async fn send_as(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
        headers: &[(&str, &str)],
    ) -> Reply {
        let method = reqwest::Method::from_bytes(method.as_bytes()).unwrap();
        let mut request = self
            .client
            .request(method, self.url(path))
            .bearer_auth(&self.token);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        if let Some(body) = body {
            request = request.json(&body);
        }
        reply(request.send().await.unwrap()).await
    }

    /// As the agent `claude-code`.
    pub async fn send_for(&self, method: &str, path: &str, body: Option<Value>) -> Reply {
        self.send_as(method, path, body, &[("Gentbox-Agent", "claude-code")])
            .await
    }

    pub async fn post_message(&self, fields: Value) -> Reply {
        self.send("POST", "/v1/messages", Some(message(fields)))
            .await
    }

    pub async fn poll(&self, id: &str, wait_seconds: u64) -> Value {
        let reply = self
            .send(
                "GET",
                &format!("/v1/messages/{id}/response?wait={wait_seconds}"),
                None,
            )
            .await;
        assert_eq!(reply.status, 200, "{reply:?}");
        reply.body
    }

    pub async fn stop(mut self) {
        if let Some(ingest) = self.ingest.take() {
            ingest.shutdown().await;
        }
        if let Some(hub) = self.hub.take() {
            hub.shutdown().await;
        }
    }
}

pub async fn reply(response: reqwest::Response) -> Reply {
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    let bytes = response.bytes().await.unwrap();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or(Value::String(String::from_utf8_lossy(&bytes).into_owned()))
    };
    Reply {
        status,
        headers,
        body,
    }
}

/// A valid message with the given fields merged in.
pub fn message(fields: Value) -> Value {
    let mut base = json!({
        "version": 1,
        "source": { "agent": "claude-code", "displayName": "Claude Code" },
        "conversation": { "id": "conversation-1", "cwd": "/Users/me/Developer/gentbox" },
        "title": "Ingest tests pass"
    });
    if let (Some(target), Value::Object(fields)) = (base.as_object_mut(), fields) {
        for (key, value) in fields {
            target.insert(key, value);
        }
    }
    base
}

/// A blocking prompt whose sender waits.
pub fn blocking_prompt() -> Value {
    json!({
        "type": "decision_required",
        "blocking": "blocking",
        "title": "Allow Bash to run the ingest tests?",
        "awaitsResponse": true,
        "actions": [
            { "id": "allow", "label": "Allow", "kind": "approve" },
            { "id": "deny", "label": "Deny", "kind": "deny" }
        ]
    })
}

/// An example file from `docs/schema/examples`.
pub fn example(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../docs/schema/examples")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// One raw HTTP/1.1 request over the Unix socket.
pub async fn socket_request(
    socket: &Path,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<Value>,
) -> (u16, Value) {
    let stream = tokio::net::UnixStream::connect(socket).await.unwrap();
    let (mut sender, connection) =
        hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(stream))
            .await
            .unwrap();
    tokio::spawn(connection);
    let mut request = hyper::Request::builder().method(method).uri(path);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let bytes = body.map_or_else(Bytes::new, |body| Bytes::from(body.to_string()));
    if !bytes.is_empty() {
        request = request.header("content-type", "application/json");
    }
    let response = tokio::time::timeout(
        Duration::from_secs(10),
        sender.send_request(request.body(Full::new(bytes)).unwrap()),
    )
    .await
    .unwrap()
    .unwrap();
    let status = response.status().as_u16();
    let collected = response.into_body().collect().await.unwrap().to_bytes();
    let value = serde_json::from_slice(&collected).unwrap_or(Value::Null);
    (status, value)
}

pub fn id_of(reply: &Reply) -> String {
    reply.body["id"].as_str().unwrap().to_owned()
}
