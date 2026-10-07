//! One Rekindle node per data root.
//!
//! `NodeLock` holds an exclusive lock on `<root>/node.lock` for the life of
//! the process. `rekindled` and the desktop both take it on
//! [`DataRoot::data`](crate::paths::DataRoot), so they refuse to run two
//! Veilid nodes over one set of files (plan C5, `frontend-decoupling.md`
//! F3). `std::fs::File::try_lock` is `flock(LOCK_EX)` on Unix and
//! `LockFileEx(LOCKFILE_EXCLUSIVE_LOCK)` on Windows, and the lock is released
//! when the handle closes — so a crashed daemon leaves no stale lock
//! (std docs; `evidence/c1-windows-ipc-research.md`).
//!
//! The lock is taken before the vault, the database, `veilid/` or the bus
//! socket are touched. Only its holder may then remove a leftover socket:
//! a second node is refused here and never reaches `BusServer::bind`, so
//! it cannot take a live daemon's socket.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// The lock file's name inside the data root.
pub const LOCK_FILE: &str = "node.lock";

/// An exclusive hold on a data root, released on drop.
#[derive(Debug)]
pub struct NodeLock {
    _file: File,
}

/// Why the lock could not be taken.
#[derive(Debug, thiserror::Error)]
pub enum NodeLockError {
    #[error(
        "another Rekindle node (rekindled or the desktop app) is running on {root} (pid {pid})"
    )]
    Held { root: PathBuf, pid: String },
    #[error("node lock {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

impl NodeLock {
    /// Take the exclusive lock on `root`, recording this process's pid in
    /// the lock file for the refusal message a second daemon prints.
    ///
    /// # Errors
    /// [`NodeLockError::Held`] when another process holds it; I/O errors
    /// otherwise.
    pub fn acquire(root: &Path) -> Result<Self, NodeLockError> {
        let path = root.join(LOCK_FILE);
        let io = |source| NodeLockError::Io {
            path: path.clone(),
            source,
        };
        std::fs::create_dir_all(root).map_err(io)?;
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(io)?;
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                let mut pid = String::new();
                file.read_to_string(&mut pid).map_err(io)?;
                return Err(NodeLockError::Held {
                    root: root.to_path_buf(),
                    pid: pid.trim().to_string(),
                });
            }
            Err(TryLockError::Error(source)) => return Err(io(source)),
        }
        file.set_len(0).map_err(io)?;
        file.seek(SeekFrom::Start(0)).map_err(io)?;
        write!(file, "{}", std::process::id()).map_err(io)?;
        file.sync_all().map_err(io)?;
        Ok(Self { _file: file })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The contract: one lock per data root. A second holder is refused
    /// and told who holds it; once the first is dropped, the root is free.
    #[test]
    fn second_holder_is_refused_until_the_first_drops() {
        let root = tempfile::tempdir().unwrap();
        let first = NodeLock::acquire(root.path()).unwrap();
        match NodeLock::acquire(root.path()) {
            Err(NodeLockError::Held { pid, .. }) => {
                assert_eq!(pid, std::process::id().to_string());
            }
            other => panic!("expected Held, got {other:?}"),
        }
        drop(first);
        NodeLock::acquire(root.path()).unwrap();
    }

    #[test]
    fn different_roots_do_not_contend() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let _a = NodeLock::acquire(a.path()).unwrap();
        let _b = NodeLock::acquire(b.path()).unwrap();
    }
}
