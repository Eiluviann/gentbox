//! `GET /v1/messages/{id}/response?wait=<s>`: holds up to `wait` seconds (at
//! most 55, default 0) and answers `actionResponse`, `pending` at the
//! deadline (architecture §8, ADR 0002). A hub shutting down answers
//! `pending` at once; the sender polls again and finds the hub gone.

use std::time::Duration;

use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use gentbox_core::response::ActionResponse;
use gentbox_hub::{MAX_WAIT, WaitStart};
use serde::Deserialize;

use crate::error::ApiError;
use crate::local_routes::{ApiPath, ApiQuery, ApiState, message_id_from_path};

/// The query of a long-poll. Other parameters (a client's cache-buster) are
/// ignored.
#[derive(Debug, Deserialize)]
pub(crate) struct WaitQuery {
    /// Seconds to hold; capped at 55.
    #[serde(default)]
    wait: Option<String>,
}

pub(crate) async fn response(
    State(state): State<ApiState>,
    ApiPath(id): ApiPath<String>,
    ApiQuery(query): ApiQuery<WaitQuery>,
) -> Result<Response, ApiError> {
    let id = message_id_from_path(&id)?;
    let wait = wait_duration(query.wait.as_deref())?;
    let start = state.hub.begin_wait(id.clone()).await?;
    let response = match start {
        WaitStart::Ready(response) => response,
        WaitStart::Waiting(pending) => {
            let mut shutdown = state.shutdown.clone();
            tokio::select! {
                response = pending.wait(wait) => response,
                _ = shutdown.wait_for(|is_stopping| *is_stopping) => pending_response(id),
            }
        }
    };
    Ok(Json(response).into_response())
}

/// `wait` in whole seconds: absent means 0, more than 55 is held for 55.
fn wait_duration(wait: Option<&str>) -> Result<Duration, ApiError> {
    let Some(wait) = wait else {
        return Ok(Duration::ZERO);
    };
    let seconds: u64 = wait.parse().map_err(|_| {
        ApiError::bad_request("invalid_wait", "`wait` is a whole number of seconds")
    })?;
    Ok(Duration::from_secs(seconds).min(MAX_WAIT))
}

fn pending_response(message_id: gentbox_core::ids::MessageId) -> ActionResponse {
    ActionResponse {
        message_id,
        status: gentbox_core::response::ResponseStatus::Pending,
        rescind_reason: None,
        action_id: None,
        action_kind: None,
        value: None,
        answered_via: None,
        answered_at: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wait_defaults_to_zero_and_is_capped_at_fifty_five_seconds() {
        assert_eq!(wait_duration(None).ok(), Some(Duration::ZERO));
        assert_eq!(
            wait_duration(Some("10")).ok(),
            Some(Duration::from_secs(10))
        );
        assert_eq!(wait_duration(Some("600")).ok(), Some(MAX_WAIT));
        assert!(wait_duration(Some("-1")).is_err());
        assert!(wait_duration(Some("1.5")).is_err());
    }
}
