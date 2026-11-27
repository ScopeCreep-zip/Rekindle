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
