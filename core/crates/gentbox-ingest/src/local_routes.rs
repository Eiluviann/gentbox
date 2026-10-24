//! The local router (architecture §8): the same routes on the Unix socket and
//! on loopback TCP. Routes only translate HTTP to hub commands.
//!
//! Every route but `hello` needs the sender token. The sender token can post,
//! wait, rescind, report progress and manage topics and threads; **no route
//! answers or lists messages** (answers come from the UI through the hub
//! handle).
//!
//! Routes that act for an agent without naming it in the path or body take
//! its id in the `Gentbox-Agent` header (and optionally the conversation in
//! `Gentbox-Conversation`): an agent withdraws and moves only its own
//! messages, and new topics and threads record who created them.

use std::sync::Arc;

use axum::extract::{FromRequest, Path, Query, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router, middleware};
use gentbox_core::ids::{ConversationKey, MessageId, ThreadId, TopicId};
use gentbox_core::message::{Conversation, MessageInput};
use gentbox_core::rescind::{RescindReason, RescindRequest};
use gentbox_core::response::IngestWarning;
use gentbox_core::topic::{CloseRequest, GroupStatus, ThreadCreate, TopicCreate};
use gentbox_hub::{
    HubHandle, IngestStatus, MessageThreadMove, Requester, ThreadMove, ThreadUpdate, TopicUpdate,
    Upserted,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::watch;

use crate::auth::{check_host, require_token};
use crate::error::ApiError;
use crate::hello::hello;
use crate::limits::RateLimiter;
use crate::long_poll;
use crate::schema::{BodyShape, validate};

/// The header naming the agent a request acts for.
pub const AGENT_HEADER: &str = "gentbox-agent";
/// The header naming the conversation a request acts for (optional).
pub const CONVERSATION_HEADER: &str = "gentbox-conversation";

/// What every handler shares.
#[derive(Clone)]
pub struct ApiState {
    pub(crate) hub: HubHandle,
    pub(crate) token: Arc<str>,
    pub(crate) limits: Arc<RateLimiter>,
    pub(crate) shutdown: watch::Receiver<bool>,
}

impl ApiState {
    /// The state of a server: the hub, the sender token, the limiter and a
    /// shutdown signal (long-polls answer `pending` when it turns true).
    pub fn new(
        hub: HubHandle,
        token: &str,
        limits: Arc<RateLimiter>,
        shutdown: watch::Receiver<bool>,
    ) -> Self {
        Self {
            hub,
            token: Arc::from(token),
            limits,
            shutdown,
        }
    }
}

/// Which listener a router serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    /// The Unix socket: no `Host` check.
    UnixSocket,
    /// Loopback TCP on this port: `Host` must name it.
    Tcp {
        /// The bound port.
        port: u16,
    },
}

/// The local router for one listener.
pub fn local_router(state: ApiState, transport: Transport) -> Router {
    let authenticated = Router::new()
        .route("/v1/messages", post(post_message))
        .route("/v1/messages/{id}", axum::routing::delete(delete_message))
        .route("/v1/messages/{id}/response", get(long_poll::response))
        .route("/v1/messages/{id}/thread", put(move_message))
        .route("/v1/conversations/{agent}/{id}", put(put_conversation))
        .route(
            "/v1/conversations/{agent}/{id}/rescind",
            post(rescind_conversation),
        )
        .route("/v1/conversations/{agent}/{id}/progress", post(progress))
        .route("/v1/presence/focus", post(presence_focus))
        .route("/v1/topics", post(create_topic).get(list_topics))
        .route("/v1/topics/{id}", get(get_topic).patch(patch_topic))
        .route("/v1/topics/{id}/close", post(close_topic))
        .route("/v1/topics/{id}/reopen", post(reopen_topic))
        .route("/v1/topics/{id}/rescind", post(rescind_topic))
        .route("/v1/threads", post(create_thread).get(list_threads))
        .route("/v1/threads/{id}", get(get_thread).patch(patch_thread))
        .route("/v1/threads/{id}/close", post(close_thread))
        .route("/v1/threads/{id}/reopen", post(reopen_thread))
        .route("/v1/threads/{id}/rescind", post(rescind_thread))
        .route("/v1/threads/{id}/topic", put(move_thread))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_token));
    let router = Router::new()
        .route("/v1/hello", post(hello))
        .merge(authenticated)
        .fallback(not_found)
        .with_state(state);
    match transport {
        Transport::UnixSocket => router,
        Transport::Tcp { port } => router.layer(middleware::from_fn_with_state(port, check_host)),
    }
}

async fn not_found() -> ApiError {
    ApiError::not_found("no such route")
}

// ---------------------------------------------------------------------
// Bodies
// ---------------------------------------------------------------------

/// A request body: `application/json`, at most the body limit (413), parsed
/// (400 on bad JSON). An empty body is allowed where a route says so.
#[derive(Debug)]
pub struct JsonBody(Option<Value>);

impl FromRequest<ApiState> for JsonBody {
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &ApiState) -> Result<Self, ApiError> {
        let limit = state.limits.limits().body_bytes;
        let is_json = request
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(is_json_content_type);
        let declared_length = request
            .headers()
            .get(header::CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<usize>().ok());
        if declared_length.is_some_and(|length| length > limit) {
            return Err(too_large(limit));
        }
        let bytes = axum::body::to_bytes(request.into_body(), limit)
            .await
            .map_err(|_| too_large(limit))?;
        if bytes.is_empty() {
            return Ok(Self(None));
        }
        if !is_json {
            return Err(ApiError::new(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "unsupported_media_type",
                "the body must be `Content-Type: application/json`",
            ));
        }
        let value = serde_json::from_slice(&bytes)
            .map_err(|error| ApiError::bad_request("invalid_json", error.to_string()))?;
        Ok(Self(Some(value)))
    }
}

impl JsonBody {
    /// Validates the body against the schema, then reads it.
    ///
    /// # Errors
    ///
    /// 422 `schema_invalid`, also for a missing body.
    pub fn parse<T: DeserializeOwned>(self, shape: BodyShape) -> Result<T, ApiError> {
        let value = self
            .0
            .ok_or_else(|| ApiError::schema_invalid("", "a JSON body is required"))?;
        validate(shape, &value)?;
        read(value)
    }

    /// Validates a body that may be absent.
    ///
    /// # Errors
    ///
    /// 422 `schema_invalid`.
    pub fn parse_optional<T: DeserializeOwned>(
        self,
        shape: BodyShape,
    ) -> Result<Option<T>, ApiError> {
        match self.0 {
            None => Ok(None),
            Some(value) => Self(Some(value)).parse(shape).map(Some),
        }
    }

    /// Reads a body with no schema entry of its own (its type checks it).
    ///
    /// # Errors
    ///
    /// 422 `schema_invalid`.
    pub fn parse_unvalidated<T: DeserializeOwned>(self) -> Result<T, ApiError> {
        let value = self
            .0
            .ok_or_else(|| ApiError::schema_invalid("", "a JSON body is required"))?;
        read(value)
    }

    fn is_absent_or_empty_object(&self) -> bool {
        match &self.0 {
            None => true,
            Some(Value::Object(fields)) => fields.is_empty(),
            Some(_) => false,
        }
    }
}

fn read<T: DeserializeOwned>(value: Value) -> Result<T, ApiError> {
    serde_json::from_value(value).map_err(|error| ApiError::schema_invalid("", error.to_string()))
}

fn is_json_content_type(value: &str) -> bool {
    value
        .split(';')
        .next()
        .is_some_and(|media_type| media_type.trim().eq_ignore_ascii_case("application/json"))
}

fn too_large(limit: usize) -> ApiError {
    ApiError::new(
        StatusCode::PAYLOAD_TOO_LARGE,
        "too_large",
        format!("the body is larger than {limit} bytes"),
    )
}

// ---------------------------------------------------------------------
// Path, query and header values
// ---------------------------------------------------------------------

fn is_ulid(text: &str) -> bool {
    text.len() == 26
        && text.bytes().all(|byte| {
            byte.is_ascii_digit() || (byte.is_ascii_uppercase() && !b"ILOU".contains(&byte))
        })
}

/// A message id from the path; anything but a ULID is a 404.
pub(crate) fn message_id_from_path(id: &str) -> Result<MessageId, ApiError> {
    if is_ulid(id) {
        Ok(MessageId::new(id))
    } else {
        Err(ApiError::not_found("no message has this id"))
    }
}

fn topic_id_from_path(id: &str) -> Result<TopicId, ApiError> {
    is_ulid(id)
        .then(|| TopicId::new(id))
        .ok_or_else(|| ApiError::not_found("no topic has this id"))
}

fn thread_id_from_path(id: &str) -> Result<ThreadId, ApiError> {
    is_ulid(id)
        .then(|| ThreadId::new(id))
        .ok_or_else(|| ApiError::not_found("no thread has this id"))
}

/// `source.agent`: `^[a-z0-9][a-z0-9-]{0,63}$`.
fn is_agent(text: &str) -> bool {
    let bytes = text.as_bytes();
    (1..=64).contains(&bytes.len())
        && (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

/// The schema's `token`: `^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$`.
fn is_token(text: &str) -> bool {
    let bytes = text.as_bytes();
    (1..=128).contains(&bytes.len())
        && bytes[0].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:-".contains(byte))
}

fn conversation_from_path(agent: &str, id: &str) -> Result<ConversationKey, ApiError> {
    if !is_agent(agent) || !is_token(id) {
        return Err(ApiError::bad_request(
            "invalid_path",
            "the agent or conversation id in the path is not valid",
        ));
    }
    Ok(ConversationKey::new(agent, id))
}

/// The requester from `Gentbox-Agent` (required) and `Gentbox-Conversation`.
fn requester(headers: &HeaderMap) -> Result<Requester, ApiError> {
    let header_text = |name| headers.get(name).and_then(|value| value.to_str().ok());
    let agent = header_text(AGENT_HEADER)
        .filter(|agent| is_agent(agent))
        .ok_or_else(|| {
            ApiError::bad_request(
                "agent_required",
                "this route needs the `Gentbox-Agent` header with the sender's source.agent",
            )
        })?;
    let conversation_id = match header_text(CONVERSATION_HEADER) {
        None => None,
        Some(id) if is_token(id) => Some(id.to_owned()),
        Some(_) => {
            return Err(ApiError::bad_request(
                "invalid_conversation",
                "`Gentbox-Conversation` is not a valid conversation id",
            ));
        }
    };
    Ok(Requester {
        agent: agent.to_owned(),
        conversation_id,
    })
}

fn status_from_query(status: Option<&str>) -> Result<Option<GroupStatus>, ApiError> {
    match status {
        None => Ok(None),
        Some("open") => Ok(Some(GroupStatus::Open)),
        Some("closed") => Ok(Some(GroupStatus::Closed)),
        Some(_) => Err(ApiError::bad_request(
            "invalid_status",
            "`status` is `open` or `closed`",
        )),
    }
}

fn upserted<T: Serialize>(record: Upserted<T>) -> Response {
    match record {
        Upserted::Created(record) => (StatusCode::CREATED, Json(record)).into_response(),
        Upserted::Existing(record) => (StatusCode::OK, Json(record)).into_response(),
    }
}

/// `{ "rescinded": [ids] }`.
#[derive(Debug, Serialize)]
struct Rescinded {
    rescinded: Vec<MessageId>,
}

fn rescinded(ids: Vec<MessageId>) -> Response {
    Json(Rescinded { rescinded: ids }).into_response()
}

// ---------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------

async fn post_message(State(state): State<ApiState>, body: JsonBody) -> Result<Response, ApiError> {
    let value = body
        .0
        .ok_or_else(|| ApiError::schema_invalid("", "a JSON body is required"))?;
    // The limit is per agent and conversation, so it reads them before the
    // full validation; a body without them fails validation anyway.
    // Only well-formed ids get buckets: anything else fails validation below
    // without growing the limiter.
    let agent = value["source"]["agent"].as_str().unwrap_or_default();
    let conversation = value["conversation"]["id"].as_str().unwrap_or_default();
    if is_agent(agent)
        && is_token(conversation)
        && let Err(retry_after) = state.limits.check_message(agent, conversation)
    {
        return Err(ApiError::rate_limited(retry_after));
    }
    let input: MessageInput = JsonBody(Some(value)).parse(BodyShape::Message)?;
    let outcome = state.hub.ingest(input).await?;
    let status = match outcome.status {
        IngestStatus::Created => StatusCode::CREATED,
        IngestStatus::Existing => StatusCode::OK,
    };
    Ok((status, Json(outcome.response)).into_response())
}

async fn delete_message(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: JsonBody,
) -> Result<Response, ApiError> {
    let id = message_id_from_path(&id)?;
    let requester = requester(&headers)?;
    let request: Option<RescindRequest> =
        body.parse_optional(BodyShape::Definition("rescindRequest"))?;
    if request
        .as_ref()
        .is_some_and(|request| request.correlation_keys.is_some())
    {
        return Err(ApiError::schema_invalid(
            "/correlationKeys",
            "DELETE /v1/messages/{id} takes a reason only; rescind by key on the conversation",
        ));
    }
    let reason = request.map_or(RescindReason::ResolvedElsewhere, |request| request.reason);
    let response = state.hub.rescind(id, reason, requester).await?;
    Ok(Json(response).into_response())
}

/// `{ messageId, topicId, threadId }`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MessagePlace {
    message_id: MessageId,
    topic_id: TopicId,
    thread_id: ThreadId,
}

async fn move_message(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: JsonBody,
) -> Result<Response, ApiError> {
    let id = message_id_from_path(&id)?;
    let requester = requester(&headers)?;
    let request: MessageThreadMove = body.parse(BodyShape::Definition("messageThreadMove"))?;
    let place = state.hub.move_message(id, request, requester).await?;
    Ok(Json(MessagePlace {
        message_id: place.message_id,
        topic_id: place.topic_id,
        thread_id: place.thread_id,
    })
    .into_response())
}

// ---------------------------------------------------------------------
// Conversations and presence
// ---------------------------------------------------------------------

/// The answer to `PUT /v1/conversations/{agent}/{id}`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConversationRecord {
    agent: String,
    conversation: Conversation,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<IngestWarning>,
}

async fn put_conversation(
    State(state): State<ApiState>,
    Path((agent, id)): Path<(String, String)>,
    body: JsonBody,
) -> Result<Response, ApiError> {
    let key = conversation_from_path(&agent, &id)?;
    let conversation: Conversation = body.parse(BodyShape::Definition("conversation"))?;
    if conversation.id != key.conversation_id {
        return Err(ApiError::schema_invalid(
            "/id",
            "the body's id must be the conversation id in the path",
        ));
    }
    let update = state
        .hub
        .upsert_conversation(key.clone(), conversation)
        .await?;
    Ok(Json(ConversationRecord {
        agent: key.agent,
        conversation: update.conversation,
        warnings: update.warnings,
    })
    .into_response())
}

async fn rescind_conversation(
    State(state): State<ApiState>,
    Path((agent, id)): Path<(String, String)>,
    body: JsonBody,
) -> Result<Response, ApiError> {
    let key = conversation_from_path(&agent, &id)?;
    let request: RescindRequest = body.parse(BodyShape::Definition("rescindRequest"))?;
    let ids = state.hub.rescind_by_correlation_keys(key, request).await?;
    Ok(rescinded(ids))
}

async fn progress(
    State(state): State<ApiState>,
    Path((agent, id)): Path<(String, String)>,
    body: JsonBody,
) -> Result<Response, ApiError> {
    let key = conversation_from_path(&agent, &id)?;
    if !body.is_absent_or_empty_object() {
        return Err(ApiError::schema_invalid(
            "",
            "progress takes no body, or an empty object",
        ));
    }
    let ids = state.hub.progress(key).await?;
    Ok(rescinded(ids))
}

/// Body of `POST /v1/presence/focus`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FocusUpdate {
    focused: bool,
}

async fn presence_focus(
    State(state): State<ApiState>,
    body: JsonBody,
) -> Result<Response, ApiError> {
    let update: FocusUpdate = body.parse_unvalidated()?;
    state.hub.set_focus(update.focused).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

// ---------------------------------------------------------------------
// Topics
// ---------------------------------------------------------------------

/// `?status=open|closed` and, for threads, `?topic=<id>`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GroupQuery {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    topic: Option<String>,
}

/// `{ "topics": [ … ] }`.
#[derive(Debug, Serialize)]
struct TopicList {
    topics: Vec<gentbox_core::topic::TopicRecord>,
}

/// `{ "threads": [ … ] }`.
#[derive(Debug, Serialize)]
struct ThreadList {
    threads: Vec<gentbox_core::topic::ThreadRecord>,
}

async fn create_topic(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: JsonBody,
) -> Result<Response, ApiError> {
    let requester = requester(&headers)?;
    let request: TopicCreate = body.parse(BodyShape::Definition("topicCreate"))?;
    Ok(upserted(
        state.hub.create_topic(requester.agent, request).await?,
    ))
}

async fn list_topics(
    State(state): State<ApiState>,
    Query(query): Query<GroupQuery>,
) -> Result<Response, ApiError> {
    if query.topic.is_some() {
        return Err(ApiError::bad_request(
            "invalid_query",
            "topics are filtered by `status` only",
        ));
    }
    let status = status_from_query(query.status.as_deref())?;
    let topics = state.hub.list_topics(status).await?;
    Ok(Json(TopicList { topics }).into_response())
}

async fn get_topic(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let id = topic_id_from_path(&id)?;
    Ok(Json(state.hub.topic(id).await?).into_response())
}

async fn patch_topic(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    body: JsonBody,
) -> Result<Response, ApiError> {
    let id = topic_id_from_path(&id)?;
    let update: TopicUpdate = body.parse(BodyShape::Definition("topicUpdate"))?;
    Ok(Json(state.hub.update_topic(id, update).await?).into_response())
}

async fn close_topic(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: JsonBody,
) -> Result<Response, ApiError> {
    let id = topic_id_from_path(&id)?;
    let requester = requester(&headers)?;
    let request: CloseRequest = body
        .parse_optional(BodyShape::Definition("closeRequest"))?
        .unwrap_or_default();
    Ok(Json(state.hub.close_topic(id, request, Some(requester)).await?).into_response())
}

async fn reopen_topic(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let id = topic_id_from_path(&id)?;
    Ok(Json(state.hub.reopen_topic(id).await?).into_response())
}

async fn rescind_topic(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: JsonBody,
) -> Result<Response, ApiError> {
    let id = topic_id_from_path(&id)?;
    let requester = requester(&headers)?;
    let reason = group_rescind_reason(body)?;
    Ok(rescinded(
        state.hub.rescind_topic(id, reason, Some(requester)).await?,
    ))
}

/// The reason of a topic or thread rescind: `{ "reason": … }`, default
/// `obsolete` as for closing.
fn group_rescind_reason(body: JsonBody) -> Result<RescindReason, ApiError> {
    let request: Option<RescindRequest> =
        body.parse_optional(BodyShape::Definition("rescindRequest"))?;
    Ok(request.map_or(RescindReason::Obsolete, |request| request.reason))
}

// ---------------------------------------------------------------------
// Threads
// ---------------------------------------------------------------------

async fn create_thread(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: JsonBody,
) -> Result<Response, ApiError> {
    let requester = requester(&headers)?;
    let request: ThreadCreate = body.parse(BodyShape::Definition("threadCreate"))?;
    Ok(upserted(
        state.hub.create_thread(requester.agent, request).await?,
    ))
}

async fn list_threads(
    State(state): State<ApiState>,
    Query(query): Query<GroupQuery>,
) -> Result<Response, ApiError> {
    let status = status_from_query(query.status.as_deref())?;
    let topic = query.topic.as_deref().map(topic_id_from_path).transpose()?;
    let threads = state.hub.list_threads(topic, status).await?;
    Ok(Json(ThreadList { threads }).into_response())
}

async fn get_thread(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let id = thread_id_from_path(&id)?;
    Ok(Json(state.hub.thread(id).await?).into_response())
}

async fn patch_thread(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    body: JsonBody,
) -> Result<Response, ApiError> {
    let id = thread_id_from_path(&id)?;
    let update: ThreadUpdate = body.parse(BodyShape::Definition("threadUpdate"))?;
    Ok(Json(state.hub.update_thread(id, update).await?).into_response())
}

async fn close_thread(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: JsonBody,
) -> Result<Response, ApiError> {
    let id = thread_id_from_path(&id)?;
    let requester = requester(&headers)?;
    let request: CloseRequest = body
        .parse_optional(BodyShape::Definition("closeRequest"))?
        .unwrap_or_default();
    Ok(Json(state.hub.close_thread(id, request, Some(requester)).await?).into_response())
}

async fn reopen_thread(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let id = thread_id_from_path(&id)?;
    Ok(Json(state.hub.reopen_thread(id).await?).into_response())
}

async fn rescind_thread(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: JsonBody,
) -> Result<Response, ApiError> {
    let id = thread_id_from_path(&id)?;
    let requester = requester(&headers)?;
    let reason = group_rescind_reason(body)?;
    Ok(rescinded(
        state
            .hub
            .rescind_thread(id, reason, Some(requester))
            .await?,
    ))
}

async fn move_thread(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: JsonBody,
) -> Result<Response, ApiError> {
    let id = thread_id_from_path(&id)?;
    let requester = requester(&headers)?;
    let request: ThreadMove = body.parse(BodyShape::Definition("threadMove"))?;
    Ok(Json(state.hub.move_thread(id, request, requester).await?).into_response())
}
