//! Gentbox ingest: the local HTTP/1.1 API (`axum` on `tokio`) on the Unix
//! socket `<state>/API/hub.sock` and on `127.0.0.1:<random port>`
//! (architecture §8, ADR 0002). Routes only translate HTTP to hub commands.
//!
//! | Module | Role |
//! | --- | --- |
//! | [`local_routes`] | the router and its handlers |
//! | `auth` | the bearer token and the `Host` check on TCP |
//! | [`hello`] | the challenge-response proof |
//! | [`limits`] | rate and body limits |
//! | `long_poll` | `GET /v1/messages/{id}/response` |
//! | [`discovery`] | the `API/` folder: token, port, socket |
//! | [`schema`] | validation against `message-v1.json` |
//! | [`error`] | error bodies and status codes |

mod auth;
pub mod discovery;
pub mod error;
pub mod hello;
pub mod limits;
pub mod local_routes;
mod long_poll;
pub mod schema;

use std::net::{Ipv4Addr, SocketAddr};
use std::path::Path;
use std::sync::Arc;

use gentbox_hub::HubHandle;
use thiserror::Error;
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio::task::JoinHandle;

pub use auth::is_token;
pub use discovery::{ApiDir, DiscoveryError, SenderToken};
pub use error::{ApiError, ErrorBody};
pub use hello::{HelloResponse, hello_proof, verify_hello_proof};
pub use limits::{Limits, RateLimiter};
pub use local_routes::{AGENT_HEADER, ApiState, CONVERSATION_HEADER, Transport, local_router};

/// How long a shutdown waits for in-flight requests.
const SHUTDOWN_GRACE: std::time::Duration = std::time::Duration::from_secs(5);

/// The longest Unix socket path that fits `sockaddr_un` on macOS (104 bytes
/// with the terminating NUL; Linux allows 108).
#[cfg(unix)]
const MAX_SOCKET_PATH_BYTES: usize = 103;

/// Why the listeners did not start.
#[derive(Debug, Error)]
pub enum IngestStartError {
    /// The discovery folder could not be prepared.
    #[error(transparent)]
    Discovery(#[from] DiscoveryError),
    /// A listener could not be bound.
    #[error("cannot listen on {what}: {source}")]
    Bind {
        /// The socket path or the TCP address.
        what: String,
        /// What failed.
        #[source]
        source: std::io::Error,
    },
}

/// The running listeners.
#[derive(Debug)]
pub struct IngestServer {
    api_dir: ApiDir,
    port: u16,
    shutdown: watch::Sender<bool>,
    tasks: Vec<JoinHandle<std::io::Result<()>>>,
}

impl IngestServer {
    /// Prepares `<state_dir>/API`, binds the Unix socket (0600) and
    /// `127.0.0.1` on a random free port, writes `port`, and serves the local
    /// router on both. Call it only while holding the hub lock (a running
    /// [`gentbox_hub::Hub`] holds it).
    ///
    /// # Errors
    ///
    /// [`IngestStartError`] when a file or a listener fails.
    pub async fn start(
        hub: HubHandle,
        state_dir: &Path,
        limits: Limits,
    ) -> Result<Self, IngestStartError> {
        let api_dir = ApiDir::prepare(state_dir)?;
        let (shutdown, shutdown_signal) = watch::channel(false);
        // The TCP port comes first: the hello proof is bound to it.
        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
        let bind_error = |source| IngestStartError::Bind {
            what: address.to_string(),
            source,
        };
        let tcp_listener = TcpListener::bind(address).await.map_err(bind_error)?;
        let port = tcp_listener.local_addr().map_err(bind_error)?.port();
        let state = ApiState::new(
            hub,
            api_dir.token(),
            port,
            Arc::new(RateLimiter::new(limits)),
            shutdown_signal.clone(),
        );
        let mut tasks = Vec::new();

        #[cfg(unix)]
        if is_socket_path_too_long(&api_dir.socket_path()) {
            // Clients fall back to TCP after the hello proof (§8).
            tracing::warn!(
                path = %api_dir.socket_path().display(),
                "the socket path is too long for a Unix socket; serving TCP only"
            );
        } else {
            let socket_path = api_dir.socket_path();
            let bind_error = |source| IngestStartError::Bind {
                what: socket_path.display().to_string(),
                source,
            };
            match tokio::net::UnixListener::bind(&socket_path) {
                Ok(listener) => {
                    discovery::set_mode(&socket_path, 0o600).map_err(bind_error)?;
                    let router = local_router(state.clone(), Transport::UnixSocket);
                    tasks.push(tokio::spawn(serve(
                        listener,
                        router,
                        shutdown_signal.clone(),
                    )));
                }
                // The Mac app's sandbox refuses a socket outside its own
                // containers (an ad hoc build's Application Support folder).
                // Clients fall back to TCP after the hello proof (§8).
                Err(error) if is_refused(&error) => tracing::warn!(
                    path = %socket_path.display(),
                    %error,
                    "the socket is not permitted here; serving TCP only"
                ),
                Err(error) => return Err(bind_error(error)),
            }
        }

        let router = local_router(state, Transport::Tcp { port });
        tasks.push(tokio::spawn(serve(tcp_listener, router, shutdown_signal)));
        api_dir.write_port(port)?;
        tracing::info!(port, api_dir = %api_dir.path().display(), "ingest listening");
        Ok(Self {
            api_dir,
            port,
            shutdown,
            tasks,
        })
    }

    /// The TCP port.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// The discovery folder.
    pub fn api_dir(&self) -> &ApiDir {
        &self.api_dir
    }

    /// Stops accepting, answers waiting long-polls `pending`, lets in-flight
    /// requests finish for up to 5 s (a stalled client cannot hold the
    /// shutdown), and removes `port` and the socket.
    pub async fn shutdown(self) {
        let _ = self.shutdown.send(true);
        for mut task in self.tasks {
            match tokio::time::timeout(SHUTDOWN_GRACE, &mut task).await {
                Ok(Ok(Err(error))) => tracing::warn!(%error, "a listener ended with an error"),
                Ok(Err(error)) => tracing::warn!(%error, "a listener task failed"),
                Ok(Ok(Ok(()))) => {}
                Err(_) => {
                    tracing::warn!("requests still open after the grace period; dropping them");
                    task.abort();
                }
            }
        }
        self.api_dir.remove_endpoints();
    }
}

async fn serve<L>(
    listener: L,
    router: axum::Router,
    mut shutdown: watch::Receiver<bool>,
) -> std::io::Result<()>
where
    L: axum::serve::Listener,
    L::Addr: std::fmt::Debug,
{
    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            let _ = shutdown.wait_for(|is_stopping| *is_stopping).await;
        })
        .await
}

#[cfg(unix)]
fn is_socket_path_too_long(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().len() > MAX_SOCKET_PATH_BYTES
}

/// A bind the operating system refuses outright (a sandbox), as opposed to a
/// failure worth stopping for.
#[cfg(unix)]
fn is_refused(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::PermissionDenied
}

#[cfg(all(test, unix))]
mod tests {
    use super::is_refused;

    #[test]
    fn only_a_refused_bind_falls_back_to_tcp() {
        // EPERM: the macOS sandbox; EACCES: a folder without write access.
        assert!(is_refused(&std::io::Error::from_raw_os_error(1)));
        assert!(is_refused(&std::io::Error::from_raw_os_error(13)));
        // A socket already in use is a real problem: another hub.
        assert!(!is_refused(&std::io::Error::from(
            std::io::ErrorKind::AddrInUse
        )));
    }
}
