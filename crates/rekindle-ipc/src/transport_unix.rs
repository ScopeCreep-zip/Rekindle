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
