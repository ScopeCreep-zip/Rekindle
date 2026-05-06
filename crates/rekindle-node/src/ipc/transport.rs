//! Transport abstractions: UCred extraction and socket path resolution.
//!
//! [RC-18] All platform gates use `#[cfg(unix)]` for Linux + macOS,
//! not `#[cfg(target_os = "linux")]`. The underlying `SO_PEERCRED` /
//! `LOCAL_PEERCRED` syscalls are available on both platforms via tokio.


use std::path::PathBuf;

use super::error::{IpcError, Result};

/// Peer credentials obtained from the Unix domain socket transport layer.
#[derive(Debug, Clone)]
pub struct PeerCredentials {
    /// Process ID of the peer.
    pub pid: u32,
    /// User ID of the peer (Unix). On Windows, this is 0.
    pub uid: u32,
}

impl PeerCredentials {
    /// Credentials for the current process (used for Noise prologue construction).
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
/// Returns an error if credentials cannot be extracted — the caller
/// MUST reject the connection. [RC-6]
#[cfg(unix)]
pub fn extract_ucred(stream: &tokio::net::UnixStream) -> Result<PeerCredentials> {
    let cred = stream
        .peer_cred()
        .map_err(IpcError::UcredFailed)?;

    let pid = cred
        .pid()
        .and_then(|p| u32::try_from(p).ok())
        .ok_or_else(|| {
            IpcError::UcredFailed(std::io::Error::other(
                "PID unavailable from UCred",
            ))
        })?;

    Ok(PeerCredentials {
        pid,
        uid: cred.uid(),
    })
}

/// Get the current process's real UID.
///
/// Uses `rustix` for safe, zero-unsafe POSIX syscall access. [RC-10]
#[cfg(unix)]
fn current_uid() -> u32 {
    rustix::process::getuid().as_raw()
}

#[cfg(not(unix))]
fn current_uid() -> u32 {
    0
}

/// Resolve the platform-appropriate IPC socket path.
///
/// [RC-5] Path is constructed from trusted system variables only
/// (`$XDG_RUNTIME_DIR`, home directory). No user-controlled path components.
pub fn socket_path() -> Result<PathBuf> {
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

    #[cfg(target_os = "windows")]
    {
        Ok(PathBuf::from(r"\\.\pipe\rekindle\daemon"))
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
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

/// Resolve the runtime directory for IPC key files.
///
/// Returns `$XDG_RUNTIME_DIR/rekindle/` on Linux.
pub fn runtime_dir() -> Result<PathBuf> {
    let sock = socket_path()?;
    // Parent of daemon.sock is the runtime directory.
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
        #[cfg(unix)]
        assert!(creds.uid < u32::MAX); // Not the sentinel value
    }
}
