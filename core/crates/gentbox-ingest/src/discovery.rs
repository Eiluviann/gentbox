//! The discovery folder (architecture §8): `<state>/API/` (0700) holds
//! `hub.sock` (0600), `token` (0600, 256-bit random, base64url) and `port`.
//! The hub removes `port` and the socket on a clean shutdown.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use thiserror::Error;

/// The folder's name inside the state folder.
pub const API_DIR_NAME: &str = "API";
/// The socket's file name.
pub const SOCKET_FILE_NAME: &str = "hub.sock";
/// The token's file name.
pub const TOKEN_FILE_NAME: &str = "token";
/// The TCP port's file name.
pub const PORT_FILE_NAME: &str = "port";

/// Length of a token: 32 random bytes as unpadded base64url.
const TOKEN_CHARS: usize = 43;

/// A discovery file could not be written or read.
#[derive(Debug, Error)]
#[error("discovery folder {path}: {source}")]
pub struct DiscoveryError {
    /// The file or folder.
    pub path: PathBuf,
    /// What failed.
    #[source]
    pub source: std::io::Error,
}

/// The sender token. Its `Debug` output is redacted, so a struct that holds
/// it can be logged without leaking it.
#[derive(Clone, PartialEq, Eq)]
pub struct SenderToken(String);

impl SenderToken {
    /// The token's text: the bearer value and the `hello` HMAC key.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for SenderToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SenderToken(<redacted>)")
    }
}

/// The prepared discovery folder and the token in it.
#[derive(Debug, Clone)]
pub struct ApiDir {
    path: PathBuf,
    token: SenderToken,
}

impl ApiDir {
    /// Creates `<state_dir>/API` as 0700 (tightening an existing one), keeps
    /// a valid existing token (clients may have read it) or writes a new one
    /// as 0600, and removes a `port` and socket left behind by a crash. Call
    /// it only while holding the hub lock.
    ///
    /// # Errors
    ///
    /// [`DiscoveryError`] when a file cannot be created, read or protected,
    /// or when `API` is a symlink.
    pub fn prepare(state_dir: &Path) -> Result<Self, DiscoveryError> {
        let path = state_dir.join(API_DIR_NAME);
        let fail = |path: &Path| {
            let path = path.to_path_buf();
            move |source| DiscoveryError { path, source }
        };
        if fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            return Err(DiscoveryError {
                path,
                source: std::io::Error::other("the API folder must not be a symlink"),
            });
        }
        fs::create_dir_all(&path).map_err(fail(&path))?;
        set_mode(&path, 0o700).map_err(fail(&path))?;
        for stale in [PORT_FILE_NAME, SOCKET_FILE_NAME] {
            remove_if_present(&path.join(stale)).map_err(fail(&path.join(stale)))?;
        }
        let token_path = path.join(TOKEN_FILE_NAME);
        let token = match read_token(&token_path) {
            Some(token) => {
                set_mode(&token_path, 0o600).map_err(fail(&token_path))?;
                token
            }
            None => {
                let token = new_token().map_err(fail(&token_path))?;
                write_private(&token_path, &token).map_err(fail(&token_path))?;
                token
            }
        };
        Ok(Self {
            path,
            token: SenderToken(token),
        })
    }

    /// The folder.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The sender token.
    pub fn token(&self) -> &str {
        self.token.as_str()
    }

    /// Where the socket goes.
    pub fn socket_path(&self) -> PathBuf {
        self.path.join(SOCKET_FILE_NAME)
    }

    /// Writes the TCP port (0600).
    ///
    /// # Errors
    ///
    /// [`DiscoveryError`] when the file cannot be written.
    pub fn write_port(&self, port: u16) -> Result<(), DiscoveryError> {
        let path = self.path.join(PORT_FILE_NAME);
        write_private(&path, &port.to_string()).map_err(|source| DiscoveryError { path, source })
    }

    /// Removes `port` and the socket: a clean shutdown.
    pub fn remove_endpoints(&self) {
        for name in [PORT_FILE_NAME, SOCKET_FILE_NAME] {
            let path = self.path.join(name);
            if let Err(error) = remove_if_present(&path) {
                tracing::warn!(path = %path.display(), %error, "cannot remove a discovery file");
            }
        }
    }
}

fn read_token(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let token = text.trim();
    let is_valid = token.len() == TOKEN_CHARS
        && token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_');
    is_valid.then(|| token.to_owned())
}

fn new_token() -> std::io::Result<String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|error| std::io::Error::other(error.to_string()))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

/// Writes a file readable only by its owner: a temp file created 0600, then
/// renamed over the target, so no reader ever sees a partial or open file.
fn write_private(path: &Path, contents: &str) -> std::io::Result<()> {
    let temporary = path.with_extension("tmp");
    remove_if_present(&temporary)?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    file.write_all(contents.as_bytes())?;
    file.sync_all()?;
    fs::rename(&temporary, path)?;
    set_mode(path, 0o600)
}

fn remove_if_present(path: &Path) -> std::io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

/// Sets Unix permissions (the hub runs on macOS and Linux).
#[cfg(unix)]
pub(crate) fn set_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

/// Elsewhere there are no Unix permissions to set.
#[cfg(not(unix))]
pub(crate) fn set_mode(_path: &Path, _mode: u32) -> std::io::Result<()> {
    Ok(())
}
