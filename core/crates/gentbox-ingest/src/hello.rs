//! `POST /v1/hello`: the hub proves it holds the token before a TCP client
//! sends it (architecture §8). The one route without the bearer token.
//!
//! The client sends a fresh random `nonce`; the hub answers
//!
//! ```text
//! proof = base64url(HMAC-SHA256(key = token, "gentbox-hello-v1\0" + port + "\0" + nonce))
//! ```
//!
//! where the key is the token file's text, `port` the hub's TCP port in
//! decimal (the one in `API/port`, also over the socket) and `\0` a zero
//! byte. Binding the port means a process squatting on a stale port cannot
//! relay a client's nonce to the real hub and pass its answer off; the
//! prefix keeps the proof from meaning anything elsewhere. Clients compare in
//! constant time ([`verify_hello_proof`]).

use axum::Json;
use axum::extract::{Request, State};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use subtle::ConstantTimeEq;

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

/// What the proof signs, so it means nothing outside `hello`.
const PROOF_DOMAIN: &[u8] = b"gentbox-hello-v1\0";

/// The proof for a nonce sent to the hub on `port`: what the hub answers and
/// what a client recomputes.
pub fn hello_proof(token: &str, port: u16, nonce: &str) -> String {
    URL_SAFE_NO_PAD.encode(proof_bytes(token, port, nonce))
}

/// Whether `proof` is the hub's answer for this token, port and nonce,
/// compared in constant time. A client sends its token only after this.
pub fn verify_hello_proof(token: &str, port: u16, nonce: &str, proof: &str) -> bool {
    let Ok(presented) = URL_SAFE_NO_PAD.decode(proof) else {
        return false;
    };
    let expected = proof_bytes(token, port, nonce);
    presented.ct_eq(expected.as_slice()).into()
}

fn proof_bytes(token: &str, port: u16, nonce: &str) -> Vec<u8> {
    // HMAC accepts keys of any length; this cannot fail.
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(token.as_bytes())
        .unwrap_or_else(|_| unreachable!("HMAC takes any key length"));
    mac.update(PROOF_DOMAIN);
    mac.update(port.to_string().as_bytes());
    mac.update(b"\0");
    mac.update(nonce.as_bytes());
    mac.finalize().into_bytes().to_vec()
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
        proof: hello_proof(&state.token, state.tcp_port, &request.nonce),
        hub: "gentbox".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        schema_versions: vec![gentbox_core::MESSAGE_SCHEMA_VERSION],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_proof_is_hmac_sha256_over_the_domain_the_port_and_the_nonce() {
        // The same HMAC computed by hand over the documented bytes.
        let mut mac =
            <Hmac<Sha256> as Mac>::new_from_slice(b"Jefe").unwrap_or_else(|_| unreachable!());
        mac.update(b"gentbox-hello-v1\x004242\x00what do ya want for nothing?");
        let expected = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
        assert_eq!(
            hello_proof("Jefe", 4242, "what do ya want for nothing?"),
            expected
        );
        assert!(verify_hello_proof(
            "Jefe",
            4242,
            "what do ya want for nothing?",
            &expected
        ));
        assert!(!verify_hello_proof(
            "Jefe",
            4243,
            "what do ya want for nothing?",
            &expected
        ));
        assert!(!verify_hello_proof(
            "Jefe",
            4242,
            "what do ya want for nothing?",
            "not base64!"
        ));
    }
}
