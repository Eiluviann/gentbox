//! The sender token on every route but `hello`, and the `Host` check on TCP
//! (architecture §8, §12).

use axum::extract::{Request, State};
use axum::http::header;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use subtle::ConstantTimeEq;

use crate::error::ApiError;
use crate::local_routes::ApiState;

/// Requires `Authorization: Bearer <token>`, compared in constant time.
pub(crate) async fn require_token(
    State(state): State<ApiState>,
    request: Request,
    next: Next,
) -> Response {
    let presented = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    match presented {
        Some(token) if is_token(token, &state.token) => next.run(request).await,
        _ => ApiError::unauthorized().into_response(),
    }
}

/// Whether `presented` is the hub's token, in time independent of where they
/// differ.
pub fn is_token(presented: &str, token: &str) -> bool {
    presented.as_bytes().ct_eq(token.as_bytes()).into()
}

/// On TCP, `Host` must be `127.0.0.1:<port>` or `localhost:<port>`: a page
/// that rebinds a domain to the loopback address sends its own host and is
/// refused. The Unix socket is not checked.
pub(crate) async fn check_host(State(port): State<u16>, request: Request, next: Next) -> Response {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok());
    match host {
        Some(host) if is_loopback_host(host, port) => next.run(request).await,
        _ => ApiError::bad_request(
            "bad_host",
            "the Host header must be 127.0.0.1:<port> or localhost:<port>",
        )
        .into_response(),
    }
}

fn is_loopback_host(host: &str, port: u16) -> bool {
    let expected_port = port.to_string();
    match host.rsplit_once(':') {
        Some((name, host_port)) => {
            host_port == expected_port && matches!(name, "127.0.0.1" | "localhost")
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_loopback_names_with_the_port_pass() {
        assert!(is_loopback_host("127.0.0.1:4000", 4000));
        assert!(is_loopback_host("localhost:4000", 4000));
        for host in [
            "127.0.0.1",
            "localhost",
            "127.0.0.1:4001",
            "evil.example:4000",
            "localhost.evil.example:4000",
            "[::1]:4000",
            "0.0.0.0:4000",
        ] {
            assert!(!is_loopback_host(host, 4000), "{host}");
        }
    }

    #[test]
    fn the_token_compare_is_exact() {
        assert!(is_token("abc", "abc"));
        assert!(!is_token("abd", "abc"));
        assert!(!is_token("ab", "abc"));
        assert!(!is_token("", "abc"));
    }
}
