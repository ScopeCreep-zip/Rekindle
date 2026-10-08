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
mod unix {
    use std::path::Path;

    use tokio::net::{UnixListener, UnixStream};

    use super::{IpcError, PeerCredentials, Result};

    /// A connected bus stream.
    pub type Stream = UnixStream;

    /// Extract peer credentials from a connected Unix domain socket.
    ///
    /// tokio reads `SO_PEERCRED` on Linux and `getpeereid` (UID) plus
    /// `LOCAL_PEEREPID` (PID) on macOS. A missing UID is an error — the
    /// caller MUST reject the connection [RC-6]; a missing PID is not.
    pub fn extract_ucred(stream: &UnixStream) -> Result<PeerCredentials> {
        let cred = stream.peer_cred().map_err(IpcError::UcredFailed)?;
        Ok(PeerCredentials {
            uid: cred.uid(),
            pid: cred.pid().and_then(|p| u32::try_from(p).ok()),
        })
    }

    /// The bus listener.
    pub struct Listener(UnixListener);

    impl Listener {
        /// Bind at `path`: owner-only parent directory, a stale socket
        /// removed, the socket itself `0600`. [RC-4][RC-6]
        ///
        /// Only the holder of the data root's node lock calls this, so a
        /// socket found here is never a live daemon's.
        pub fn bind(path: &Path) -> Result<Self> {
            use std::os::unix::fs::PermissionsExt;

            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| IpcError::DirectoryCreate {
                    path: parent.display().to_string(),
                    source: e,
                })?;
                std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700)).map_err(
                    |e| IpcError::DirectoryCreate {
                        path: parent.display().to_string(),
                        source: e,
                    },
                )?;
            }

            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    return Err(IpcError::SocketBind {
                        path: path.display().to_string(),
                        source: e,
                    });
                }
            }

            let bind_error = |e| IpcError::SocketBind {
                path: path.display().to_string(),
                source: e,
            };
            let listener = UnixListener::bind(path).map_err(bind_error)?;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                .map_err(bind_error)?;
            Ok(Self(listener))
        }

        /// The next connection from a process running as our own user.
        /// Connections that fail the UID check are dropped here.
        pub async fn accept(&self) -> std::io::Result<Option<(Stream, PeerCredentials)>> {
            let (stream, _addr) = self.0.accept().await?;
            let peer = match extract_ucred(&stream) {
                Ok(creds) => creds,
                Err(e) => {
                    tracing::error!(error = %e, "rejecting: UCred extraction failed");
                    return Ok(None);
                }
            };
            let my_uid = PeerCredentials::local().uid;
            if peer.uid != my_uid {
                tracing::error!(peer_uid = peer.uid, my_uid, "rejecting: UID mismatch");
                return Ok(None);
            }
            Ok(Some((stream, peer)))
        }
    }

    /// Connect to the bus at `path`, returning the server's credentials.
    pub async fn connect(path: &Path) -> Result<(Stream, PeerCredentials)> {
        let stream = UnixStream::connect(path)
            .await
            .map_err(|e| IpcError::SocketBind {
                path: path.display().to_string(),
                source: e,
            })?;
        let server = extract_ucred(&stream)?;
        Ok((stream, server))
    }
}

#[cfg(windows)]
mod windows {
    use std::path::Path;

    use interprocess::os::windows::named_pipe::pipe_mode::Bytes;
    use interprocess::os::windows::named_pipe::tokio::{DuplexPipeStream, PipeListener};
    use interprocess::os::windows::named_pipe::PipeListenerOptions;
    use interprocess::os::windows::security_descriptor::SecurityDescriptor;

    use super::{IpcError, PeerCredentials, Result};

    /// The pipe's security descriptor (SDDL): a protected DACL (`D:P`, no
    /// inherited entries) granting full access (`GA`) to the pipe's owner
    /// (`OW`, Owner Rights — "the current owner of the object") and
    /// LocalSystem (`SY`). The per-user form of the descriptor the Docker
    /// daemon puts on its pipe (`D:P(A;;GA;;;BA)(A;;GA;;;SY)`).
    pub const PIPE_SECURITY_DESCRIPTOR: &str = "D:P(A;;GA;;;OW)(A;;GA;;;SY)";

    /// A connected bus stream.
    pub type Stream = DuplexPipeStream<Bytes>;

    /// The bus listener.
    pub struct Listener(PipeListener<Bytes, Bytes>);

    impl Listener {
        /// Create the pipe at `path` with [`PIPE_SECURITY_DESCRIPTOR`],
        /// refusing remote clients.
        pub fn bind(path: &Path) -> Result<Self> {
            let bind_error = |e| IpcError::SocketBind {
                path: path.display().to_string(),
                source: e,
            };
            let sddl = widestring::U16CString::from_str(PIPE_SECURITY_DESCRIPTOR)
                .map_err(|e| bind_error(std::io::Error::other(e)))?;
            let descriptor = SecurityDescriptor::deserialize(&sddl).map_err(bind_error)?;
            let listener = PipeListenerOptions::new()
                .path(path.as_os_str())
                .security_descriptor(Some(descriptor))
                .accept_remote(false)
                .create_tokio_duplex::<Bytes>()
                .map_err(bind_error)?;
            Ok(Self(listener))
        }

        /// The next connection. The pipe's DACL has already admitted only
        /// our own account (or LocalSystem); the client's PID is recorded
        /// for the Noise prologue.
        pub async fn accept(&self) -> std::io::Result<Option<(Stream, PeerCredentials)>> {
            let stream = self.0.accept().await?;
            let pid = stream.client_process_id().ok();
            Ok(Some((stream, PeerCredentials { uid: 0, pid })))
        }
    }

    /// Connect to the bus pipe at `path`, returning the server's credentials.
    pub async fn connect(path: &Path) -> Result<(Stream, PeerCredentials)> {
        let connect_error = |e| IpcError::SocketBind {
            path: path.display().to_string(),
            source: e,
        };
        let stream = DuplexPipeStream::<Bytes>::connect_by_path(path.as_os_str())
            .await
            .map_err(connect_error)?;
        let pid = stream.server_process_id().ok();
        Ok((stream, PeerCredentials { uid: 0, pid }))
    }
}

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

/// Resolve the runtime directory for IPC key files (`bus.pub`, `bus.key`).
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
mod tests {
    use super::*;

    #[test]
    fn local_credentials_are_valid() {
        let creds = PeerCredentials::local();
        assert!(creds.pid.is_some_and(|pid| pid > 0));
        #[cfg(unix)]
        assert!(creds.uid < u32::MAX); // Not the sentinel value
    }
}
