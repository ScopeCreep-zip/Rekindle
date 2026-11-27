//! The local transport under the Noise bus: a Unix domain socket, or a
//! named pipe on Windows, plus peer credentials and path resolution.
//!
//! Access control is the operating system's, applied before a byte is read:
//! - **Unix:** the socket lives in a `0700` directory and is `0600`; every
//!   accepted connection's peer UID (`SO_PEERCRED`; `getpeereid` on macOS)
//!   must equal ours. [RC-6]
//! - **Windows:** the pipe is created with the protected DACL
//!   [`PIPE_SECURITY_DESCRIPTOR`] — full access for the pipe's owner (the
//!   account running `rekindled`) and LocalSystem, nothing for anyone else
//!   — and remote clients are refused. Microsoft documents the security
//!   descriptor as what "controls access to both client and server ends of
//!   the named pipe"; the default one grants Everyone read access, which is
//!   why it is replaced (`evidence/c1-windows-ipc-research.md`).
//!
//! [RC-18] Unix gates use `#[cfg(unix)]` for Linux and macOS alike.

#[cfg(not(windows))]
use std::path::Path;
use std::path::PathBuf;

use super::error::{IpcError, Result};

/// Peer credentials of the other end of a connection.
#[derive(Debug, Clone)]
pub struct PeerCredentials {
    /// User ID of the peer (Unix) — the access boundary, and bound into the
    /// Noise prologue. Windows has no UID; it is 0 on both ends there, and
    /// the pipe's DACL is the user boundary.
    pub uid: u32,
    /// Process ID of the peer, for logs. `None` when the OS does not report
    /// one (e.g. a peer in another PID namespace); never a reason to refuse.
    pub pid: Option<u32>,
}

impl PeerCredentials {
    /// Credentials for the current process.
    #[must_use]
    pub fn local() -> Self {
        Self {
            uid: current_uid(),
            pid: Some(std::process::id()),
        }
    }
}

/// Get the current process's real UID.
///
/// Uses `rustix` for safe, zero-unsafe POSIX syscall access. [RC-10]
#[cfg(unix)]
fn current_uid() -> u32 {
    rustix::process::getuid().as_raw()
}

#[cfg(windows)]
fn current_uid() -> u32 {
    0
}

#[cfg(unix)]
pub use unix::{connect, extract_ucred, Listener, Stream};
#[cfg(windows)]
pub use windows::{connect, Listener, Stream, PIPE_SECURITY_DESCRIPTOR};

#[cfg(unix)]
#[path = "transport_unix.rs"]
mod unix;

#[cfg(windows)]
#[path = "transport_windows.rs"]
mod windows;

/// Resolve the platform-appropriate IPC socket path.
///
/// [RC-5] Path is constructed from trusted system variables only
/// (`$XDG_RUNTIME_DIR`, home directory). No user-controlled path components.
///
/// Windows pipe names share one machine-wide namespace, so the name carries
/// a tag of the user's profile directory: each user's `rekindled` has its
/// own pipe, and the pipe's DACL admits only that user.
pub fn socket_path() -> Result<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        let runtime = std::env::var("XDG_RUNTIME_DIR").map_err(|_| IpcError::DirectoryCreate {
            path: "$XDG_RUNTIME_DIR".into(),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "XDG_RUNTIME_DIR is not set"),
        })?;
        Ok(PathBuf::from(runtime).join("rekindle/daemon.sock"))
    }

    #[cfg(target_os = "macos")]
    {
        Ok(home_dir()?.join("Library/Application Support/rekindle/daemon.sock"))
    }

    #[cfg(windows)]
    {
        let home = home_dir()?;
        let tag = blake3::hash(home.as_os_str().as_encoded_bytes()).to_hex();
        Ok(PathBuf::from(format!(r"\\.\pipe\rekindle-{}", &tag[..16])))
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        Err(IpcError::DirectoryCreate {
            path: "unknown".into(),
            source: std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "unsupported platform for IPC socket",
            ),
        })
    }
}

/// Resolve the runtime directory for the bus socket and `bus.pub`.
///
/// On Unix it is the socket's directory (`$XDG_RUNTIME_DIR/rekindle/` on
/// Linux). A Windows pipe has no directory, so there it is
/// `%LOCALAPPDATA%\rekindle\`, inside the user's profile.
pub fn runtime_dir() -> Result<PathBuf> {
    #[cfg(windows)]
    {
        let local = dirs::data_local_dir().ok_or_else(|| IpcError::DirectoryCreate {
            path: "%LOCALAPPDATA%".into(),
            source: std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "cannot determine the local app data directory",
            ),
        })?;
        Ok(local.join("rekindle"))
    }

    #[cfg(not(windows))]
    {
        let sock = socket_path()?;
        sock.parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| IpcError::DirectoryCreate {
                path: sock.display().to_string(),
                source: std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "socket path has no parent",
                ),
            })
    }
}

#[cfg(any(target_os = "macos", windows))]
fn home_dir() -> Result<PathBuf> {
    dirs::home_dir().ok_or_else(|| IpcError::DirectoryCreate {
        path: "~/".into(),
        source: std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "cannot determine home directory",
        ),
    })
}

#[cfg(test)]
#[path = "transport_tests.rs"]
mod tests;
