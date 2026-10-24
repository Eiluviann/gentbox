//! Error bodies: `{ "code": …, "message": …, "pointer"?: … }` with the HTTP
//! status each outcome maps to (architecture §8, "Error codes").

use axum::Json;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use gentbox_core::action_plan::PlanError;
use gentbox_core::lifecycle::LifecycleState;
use gentbox_core::prepare::IngestRejection;
use gentbox_core::resolve::GroupingError;
use gentbox_hub::HubError;
use serde::Serialize;

/// An error response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError {
    status: StatusCode,
    body: ErrorBody,
    retry_after_seconds: Option<u64>,
}

/// The JSON body of every error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorBody {
    /// A stable machine-readable code (`schema_invalid`, `not_owner`, …).
    pub code: &'static str,
    /// Plain words for a person or a model.
    pub message: String,
    /// The JSON pointer of the failing field, for `schema_invalid`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pointer: Option<String>,
    /// The message's state, for `already_closed`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<LifecycleState>,
    /// How many messages are still open, for `has_open_messages`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open_messages: Option<u32>,
}

impl ApiError {
    /// An error with a status, a code and a message.
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            body: ErrorBody {
                code,
                message: message.into(),
                pointer: None,
                state: None,
                open_messages: None,
            },
            retry_after_seconds: None,
        }
    }

    /// 422 `schema_invalid` at a JSON pointer.
    pub fn schema_invalid(pointer: impl Into<String>, message: impl Into<String>) -> Self {
        let mut error = Self::new(StatusCode::UNPROCESSABLE_ENTITY, "schema_invalid", message);
        error.body.pointer = Some(pointer.into());
        error
    }

    /// 400 for a malformed request (path, query, header, JSON syntax).
    pub fn bad_request(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, code, message)
    }

    /// 401 `unauthorized`.
    pub fn unauthorized() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "a valid `Authorization: Bearer <token>` header is required",
        )
    }

    /// 404 `not_found`.
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", message)
    }

    /// 429 `rate_limited` with `Retry-After`.
    pub fn rate_limited(retry_after_seconds: u64) -> Self {
        let mut error = Self::new(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "too many requests; retry after the `Retry-After` seconds",
        );
        error.retry_after_seconds = Some(retry_after_seconds);
        error
    }

    /// The HTTP status.
    pub fn status(&self) -> StatusCode {
        self.status
    }

    /// The body.
    pub fn body(&self) -> &ErrorBody {
        &self.body
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut response = (self.status, Json(self.body)).into_response();
        if let Some(seconds) = self.retry_after_seconds {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from(seconds));
        }
        if self.status == StatusCode::UNAUTHORIZED {
            response
                .headers_mut()
                .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        }
        response
    }
}

impl From<HubError> for ApiError {
    fn from(error: HubError) -> Self {
        let message = error.to_string();
        match error {
            HubError::MessageNotFound(_)
            | HubError::TopicNotFound(_)
            | HubError::ThreadNotFound(_) => Self::not_found(message),
            HubError::NotOwner => Self::new(StatusCode::FORBIDDEN, "not_owner", message),
            HubError::MessageClosed { state, .. } => {
                let mut error = Self::new(StatusCode::CONFLICT, "already_closed", message);
                error.body.state = Some(state);
                error
            }
            HubError::MissingCorrelationKeys => Self::schema_invalid("/correlationKeys", message),
            HubError::ImplicitGroup => Self::new(StatusCode::CONFLICT, "implicit_group", message),
            HubError::HasOpenMessages { count } => {
                let mut error = Self::new(StatusCode::CONFLICT, "has_open_messages", message);
                error.body.open_messages = Some(count);
                error
            }
            HubError::GeneralTopic => Self::new(StatusCode::CONFLICT, "general_topic", message),
            HubError::Conflict(_) => Self::new(StatusCode::CONFLICT, "conflict", message),
            HubError::Rejected(rejection) => rejected(&rejection),
            HubError::Grouping(grouping) => grouping_error(&grouping),
            // Answers and the inbox are not reachable over HTTP; if one ever
            // is, it is the caller's mistake.
            HubError::UnknownAction
            | HubError::ActionNotOffered
            | HubError::InvalidAnswerValue(_)
            | HubError::UseDoneInstead
            | HubError::NotDeferrable
            | HubError::UnknownStep(_) => Self::bad_request("invalid_action", message),
            HubError::Stopped => Self::new(StatusCode::SERVICE_UNAVAILABLE, "hub_stopped", message),
            HubError::Store(_) | HubError::Panicked => {
                tracing::error!(%message, "hub command failed");
                Self::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal",
                    "the hub failed; see its log",
                )
            }
        }
    }
}

fn rejected(rejection: &IngestRejection) -> ApiError {
    let message = rejection.to_string();
    let status = StatusCode::UNPROCESSABLE_ENTITY;
    match rejection {
        IngestRejection::Invalid(invalid) => ApiError::new(status, invalid.code(), message),
        IngestRejection::TitleGuard(_) => {
            ApiError::new(status, "title_looks_like_command", message)
        }
        IngestRejection::ActionPlan(PlanError::StepsMissing) => {
            ApiError::new(status, "steps_missing", message)
        }
        IngestRejection::ActionPlan(_) => ApiError::schema_invalid("/steps", message),
    }
}

fn grouping_error(error: &GroupingError) -> ApiError {
    let message = error.to_string();
    let status = StatusCode::UNPROCESSABLE_ENTITY;
    match error {
        GroupingError::UnknownTopic(_) => ApiError::new(status, "unknown_topic", message),
        GroupingError::UnknownThread(_) => ApiError::new(status, "unknown_thread", message),
        GroupingError::TopicConflict => ApiError::new(status, "topic_conflict", message),
        GroupingError::TitleLooksLikeCommand => {
            ApiError::new(status, "title_looks_like_command", message)
        }
    }
}
