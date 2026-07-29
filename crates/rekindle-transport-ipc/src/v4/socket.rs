//! OS-level socket plumbing: UCred extraction, socket path resolution,
//! buffer tuning. Self-contained — no dependency on v1 error types or config.

use std::path::PathBuf;

#[derive(Debug)]
pub enum SocketError {
    UcredFailed(std::io::Error),
    PidUnavailable,
    DirectoryNotFound(String),
    UnsupportedPlatform,
    Io(std::io::Error),
}

impl std::fmt::Display for SocketError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UcredFailed(e) => write!(f, "SO_PEERCRED extraction failed: {e}"),
            Self::PidUnavailable => write!(f, "peer PID unavailable from UCred"),
            Self::DirectoryNotFound(p) => write!(f, "runtime directory not found: {p}"),
            Self::UnsupportedPlatform => write!(f, "unsupported platform for IPC socket"),
            Self::Io(e) => write!(f, "socket I/O error: {e}"),
        }
    }
}

/// Peer credentials latched at connect() time by the kernel's copy_peercred().
/// A peer that setuid()s or execve()s after connect retains its pre-connect identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerCredentials {
    pub pid: u32,
    pub uid: u32,
}

impl PeerCredentials {
    /// Credentials for the current process, used for Noise prologue construction.
    #[must_use]
    pub fn local() -> Self {
        Self {
            pid: std::process::id(),
            uid: current_uid(),
        }
    }
}

/// Extract peer credentials from a connected Unix domain socket via SO_PEERCRED.
#[cfg(unix)]
pub fn extract_ucred(stream: &tokio::net::UnixStream) -> Result<PeerCredentials, SocketError> {
    let cred = stream.peer_cred().map_err(SocketError::UcredFailed)?;
    let pid = cred
        .pid()
        .and_then(|p| u32::try_from(p).ok())
        .ok_or(SocketError::PidUnavailable)?;
    Ok(PeerCredentials {
        pid,
        uid: cred.uid(),
    })
}

#[cfg(unix)]
fn current_uid() -> u32 {
    rustix::process::getuid().as_raw()
}

#[cfg(not(unix))]
fn current_uid() -> u32 {
    0
}

/// Platform-appropriate IPC socket path derived from trusted system variables.
pub fn socket_path() -> Result<PathBuf, SocketError> {
    #[cfg(target_os = "linux")]
    {
        let runtime = std::env::var("XDG_RUNTIME_DIR")
            .map_err(|_| SocketError::DirectoryNotFound("$XDG_RUNTIME_DIR not set".into()))?;
        Ok(PathBuf::from(runtime).join("rekindle/daemon.sock"))
    }

    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME")
            .map_err(|_| SocketError::DirectoryNotFound("$HOME not set".into()))?;
        Ok(PathBuf::from(home).join("Library/Application Support/rekindle/daemon.sock"))
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Err(SocketError::UnsupportedPlatform)
    }
}

/// Parent directory of the socket path, used for key file storage.
pub fn runtime_dir() -> Result<PathBuf, SocketError> {
    let sock = socket_path()?;
    Ok(sock.parent().expect("socket_path always has a parent").to_owned())
}

/// Set SO_SNDBUF and SO_RCVBUF on a Unix stream. The kernel doubles the value
/// internally and clamps to wmem_max/rmem_max. Errors are silently ignored —
/// the kernel's default is acceptable if the setsockopt fails.
#[cfg(unix)]
#[allow(unsafe_code)]
pub fn apply_socket_options(
    stream: &tokio::net::UnixStream,
    sndbuf: Option<u32>,
    rcvbuf: Option<u32>,
) {
    use std::os::unix::io::AsRawFd;
    let fd = stream.as_raw_fd();
    #[allow(clippy::cast_possible_truncation)]
    let socklen = std::mem::size_of::<libc::c_int>() as libc::socklen_t;

    if let Some(val) = sndbuf {
        // SAFETY: setsockopt with SOL_SOCKET + SO_SNDBUF on a valid fd.
        unsafe {
            #[allow(clippy::cast_possible_wrap)]
            let v = val as libc::c_int;
            libc::setsockopt(
                fd, libc::SOL_SOCKET, libc::SO_SNDBUF,
                (&raw const v).cast::<libc::c_void>(), socklen,
            );
        }
    }
    if let Some(val) = rcvbuf {
        // SAFETY: setsockopt with SOL_SOCKET + SO_RCVBUF on a valid fd.
        unsafe {
            #[allow(clippy::cast_possible_wrap)]
            let v = val as libc::c_int;
            libc::setsockopt(
                fd, libc::SOL_SOCKET, libc::SO_RCVBUF,
                (&raw const v).cast::<libc::c_void>(), socklen,
            );
        }
    }
}

/// Wall-clock milliseconds since Unix epoch. Used for heartbeat timestamps,
/// event timestamps, and RTT computation.
pub fn wall_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
