//! `POST /v1/hello`: the hub proves it holds the token before a TCP client
//! sends it (architecture §8). The one route without the bearer token.
//!
//! The client sends a fresh random `nonce`; the hub answers
//! `proof = base64url(HMAC-SHA256(key = token, message = nonce))`, where the
//! key is the token file's text and the message the nonce's UTF-8 bytes. A
//! process squatting on a port left behind by a crash cannot compute it.

use axum::Json;
use axum::extract::{Request, State};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::error::ApiError;
use crate::local_routes::{ApiState, JsonBody};

/// The shortest nonce accepted: 128 bits as base64url.
pub const MIN_NONCE_CHARS: usize = 22;
/// The longest nonce accepted.
pub const MAX_NONCE_CHARS: usize = 256;

/// Body of `POST /v1/hello`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelloRequest {
    /// The client's fresh random challenge.
    pub nonce: String,
}

/// The answer to `hello`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HelloResponse {
    /// `base64url(HMAC-SHA256(token, nonce))`, no padding.
    pub proof: String,
    /// Always `gentbox`.
    pub hub: String,
    /// The hub's version.
    pub version: String,
    /// The message schema versions it accepts.
    pub schema_versions: Vec<u32>,
}

/// The proof for a nonce: what the hub answers and what a client checks.
pub fn hello_proof(token: &str, nonce: &str) -> String {
    // HMAC accepts keys of any length; this cannot fail.
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(token.as_bytes())
        .unwrap_or_else(|_| unreachable!("HMAC takes any key length"));
    mac.update(nonce.as_bytes());
    URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
}

/// A `hello` body is a nonce: anything larger than this is refused.
pub const MAX_HELLO_BODY_BYTES: usize = 1024;

/// The rate limit comes first, so an unauthenticated caller cannot make the
/// hub read anything once the bucket is empty.
pub(crate) async fn hello(State(state): State<ApiState>, request: Request) -> Response {
    if let Err(retry_after) = state.limits.check_hello() {
        return ApiError::rate_limited(retry_after).into_response();
    }
    let answer = match JsonBody::read(request, MAX_HELLO_BODY_BYTES).await {
        Ok(body) => answer(&state, body),
        Err(error) => Err(error),
    };
    match answer {
        Ok(response) => Json(response).into_response(),
        Err(error) => error.into_response(),
    }
}

fn answer(state: &ApiState, body: JsonBody) -> Result<HelloResponse, ApiError> {
    let request: HelloRequest = body.parse_unvalidated()?;
    let length = request.nonce.chars().count();
    if !(MIN_NONCE_CHARS..=MAX_NONCE_CHARS).contains(&length) {
        return Err(ApiError::schema_invalid(
            "/nonce",
            format!("the nonce must be {MIN_NONCE_CHARS} to {MAX_NONCE_CHARS} characters"),
        ));
    }
    Ok(HelloResponse {
        proof: hello_proof(&state.token, &request.nonce),
        hub: "gentbox".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        schema_versions: vec![gentbox_core::MESSAGE_SCHEMA_VERSION],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_proof_is_the_rfc_4231_hmac_sha256() {
        // RFC 4231 test case 2: key "Jefe", data "what do ya want for nothing?".
        let proof = hello_proof("Jefe", "what do ya want for nothing?");
        let bytes = URL_SAFE_NO_PAD.decode(proof).unwrap_or_default();
        let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        assert_eq!(
            hex,
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }
}
