//! The hub lock (architecture §3): one hub per user. Each hub takes an
//! exclusive lock on `<state>/hub.lock` before it opens the store or binds
//! anything; a second hub fails naming the holder.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use thiserror::Error;

/// The lock file's name inside the state folder.
pub const LOCK_FILE_NAME: &str = "hub.lock";

/// Why the lock was not taken.
#[derive(Debug, Error)]
pub enum LockError {
    /// Another hub holds the lock.
    #[error("another hub is running (pid {})", holder_pid.map_or_else(|| "unknown".to_owned(), |pid| pid.to_string()))]
    Held {
        /// The holder's process id, as it wrote it into the lock file.
        holder_pid: Option<u32>,
    },
    /// The lock file could not be opened or written.
    #[error("cannot use the hub lock at {path}: {source}")]
    Io {
        /// The lock file.
        path: PathBuf,
        /// What failed.
        #[source]
        source: std::io::Error,
    },
}

/// An exclusive lock on the state folder, released on drop (or when the
/// process dies: the operating system drops the lock with the file).
#[derive(Debug)]
pub struct HubLock {
    file: File,
    path: PathBuf,
}

impl HubLock {
    /// Takes the lock on `<state_dir>/hub.lock` without waiting, creating the
    /// folder and the file when missing, and writes this process's id into it.
    ///
    /// # Errors
    ///
    /// [`LockError::Held`] when another hub has it; [`LockError::Io`] when the
    /// file cannot be used.
    pub fn acquire(state_dir: &Path) -> Result<Self, LockError> {
        let path = state_dir.join(LOCK_FILE_NAME);
        let io_error = |source| LockError::Io {
            path: path.clone(),
            source,
        };
        std::fs::create_dir_all(state_dir).map_err(io_error)?;
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(io_error)?;
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                return Err(LockError::Held {
                    holder_pid: read_pid(&mut file),
                });
            }
            Err(TryLockError::Error(source)) => return Err(io_error(source)),
        }
        write_pid(&mut file).map_err(io_error)?;
        Ok(Self { file, path })
    }

    /// The lock file.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for HubLock {
    fn drop(&mut self) {
        // Closing the file releases the lock as well; unlocking first makes
        // the release immediate even if a duplicated descriptor lingers.
        let _ = self.file.unlock();
    }
}

fn read_pid(file: &mut File) -> Option<u32> {
    let mut text = String::new();
    file.seek(SeekFrom::Start(0)).ok()?;
    file.read_to_string(&mut text).ok()?;
    text.trim().parse().ok()
}

fn write_pid(file: &mut File) -> std::io::Result<()> {
    file.set_len(0)?;
    file.seek(SeekFrom::Start(0))?;
    writeln!(file, "{}", std::process::id())?;
    file.sync_all()
}
