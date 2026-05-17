//! OS-level socket plumbing: UCred extraction, socket path resolution.
//!
//! SO_PEERCRED latches credentials at connect() time — post-connect
//! setuid/exec is invisible to the server. Documented on the type.

use std::path::PathBuf;

use crate::error::{IpcError, IpcResult};

/// Peer credentials obtained from the Unix domain socket transport layer.
///
/// Values are latched at `connect()` time (server side) and `listen()` time
/// (client side) per the kernel's `copy_peercred()` in `unix_stream_connect`.
/// A process that `setuid()`s or `execve()`s after connect will still appear
/// with its pre-connect identity. Use `SO_PEERPIDFD` (kernel 6.5+) or an
/// application-layer handshake if liveness-sensitive auth is required.
#[derive(Debug, Clone, Copy)]
pub struct PeerCredentials {
    /// Process ID of the peer.
    pub pid: u32,
    /// User ID of the peer.
    pub uid: u32,
}

impl PeerCredentials {
    /// Credentials for the current process (used for Noise prologue).
    #[must_use]
    pub fn local() -> Self {
        Self {
            pid: std::process::id(),
            uid: current_uid(),
        }
    }
}

/// Extract peer credentials from a connected Unix domain socket.
///
/// Uses `SO_PEERCRED` on Linux, `LOCAL_PEERCRED` on macOS.
/// Returns an error if extraction fails — the caller MUST reject.
#[cfg(unix)]
pub fn extract_ucred(stream: &tokio::net::UnixStream) -> IpcResult<PeerCredentials> {
    let cred = stream.peer_cred().map_err(IpcError::UcredFailed)?;
    let pid = cred
        .pid()
        .and_then(|p| u32::try_from(p).ok())
        .ok_or_else(|| {
            IpcError::UcredFailed(std::io::Error::other("PID unavailable from UCred"))
        })?;
    Ok(PeerCredentials {
        pid,
        uid: cred.uid(),
    })
}

/// Current process real UID via rustix (safe, zero-unsafe POSIX access).
#[cfg(unix)]
fn current_uid() -> u32 {
    rustix::process::getuid().as_raw()
}

#[cfg(not(unix))]
fn current_uid() -> u32 {
    0
}

/// Platform-appropriate IPC socket path.
///
/// Constructed from trusted system variables only — no user-controlled
/// path components.
pub fn socket_path() -> IpcResult<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        let runtime = std::env::var("XDG_RUNTIME_DIR").map_err(|_| {
            IpcError::DirectoryCreate {
                path: "$XDG_RUNTIME_DIR".into(),
                source: std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "XDG_RUNTIME_DIR is not set",
                ),
            }
        })?;
        Ok(PathBuf::from(runtime).join("rekindle/daemon.sock"))
    }

    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir().ok_or_else(|| IpcError::DirectoryCreate {
            path: "~/".into(),
            source: std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "cannot determine home directory",
            ),
        })?;
        Ok(home.join("Library/Application Support/rekindle/daemon.sock"))
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
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

/// Runtime directory for IPC key files. Parent of the socket path.
pub fn runtime_dir() -> IpcResult<PathBuf> {
    let sock = socket_path()?;
    Ok(sock
        .parent()
        .expect("socket_path always has a parent")
        .to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_credentials_are_valid() {
        let creds = PeerCredentials::local();
        assert!(creds.pid > 0);
    }
}
