//! FD passing and memfd operations — production implementations.
//!
//! `LinuxTransport` wraps a `SideChannel` for SCM_RIGHTS fd passing.
//! `LinuxMemfd` wraps the `memfd` module for memfd_create + seal + mmap.
//!
//! The coordinator calls `memfd_ops.create_and_seal(data)` which returns
//! `(payload_id, hash)`. The `payload_id` is a local index into the
//! `LinuxMemfd`'s internal map of `SealedMemfd` fds. When
//! `transport.send_fd(tag)` is called with that `payload_id`, the
//! transport looks up the real fd and sends it via SCM_RIGHTS.
//!
//! On the receiver side, `transport.recv_fd()` returns a `FdTag` with
//! a `payload_id` that maps to the received fd stored in `LinuxMemfd`.
//! `memfd_ops.verify(payload_id, hash)` mmaps the fd and verifies.

use std::collections::HashMap;
use std::os::unix::io::RawFd;
use std::sync::Mutex;

use super::sidechannel::SideChannel;

/// Tag accompanying a file descriptor on the SideChannel.
#[derive(Debug, Clone)]
pub struct FdTag {
    pub stream_id: u8,
    pub flags: u8,
    pub payload_size: u64,
    pub payload_id: u32,
}

#[derive(Debug)]
pub enum TransportError {
    SendFailed(String),
    RecvFailed(String),
    Closed,
    WouldBlock,
}

#[derive(Debug)]
pub enum MemfdError {
    CreateFailed(String),
    VerifyFailed { expected: [u8; 32], computed: [u8; 32] },
    SizeMismatch { expected: u64, actual: u64 },
}

/// Platform-independent FD transport trait.
pub trait FdTransport: Send + Sync {
    fn send_fd(&self, tag: &FdTag) -> Result<(), TransportError>;
    fn recv_fd(&self) -> Result<FdTag, TransportError>;
}

/// Platform-independent memfd operations trait.
pub trait MemfdOps: Send + Sync {
    fn create_and_seal(&self, data: &[u8]) -> Result<(u32, [u8; 32]), MemfdError>;
    fn retrieve(&self, payload_id: u32) -> Option<Vec<u8>>;
    fn verify(&self, payload_id: u32, expected_hash: &[u8; 32]) -> Result<Vec<u8>, MemfdError>;
}

// ── Production implementations (Linux) ──────────────────────────

/// Production FD transport wrapping a `SideChannel`.
/// Maps `payload_id` → real `RawFd` via the shared `LinuxMemfd` fd store.
#[cfg(target_os = "linux")]
pub struct LinuxTransport {
    sidechannel: SideChannel,
    /// Shared with `LinuxMemfd` — maps payload_id to the actual RawFd.
    fd_store: std::sync::Arc<Mutex<HashMap<u32, RawFd>>>,
}

#[cfg(target_os = "linux")]
impl LinuxTransport {
    pub fn new(sidechannel: SideChannel, fd_store: std::sync::Arc<Mutex<HashMap<u32, RawFd>>>) -> Self {
        Self { sidechannel, fd_store }
    }
}

#[cfg(target_os = "linux")]
impl FdTransport for LinuxTransport {
    fn send_fd(&self, tag: &FdTag) -> Result<(), TransportError> {
        let fd = {
            let store = self.fd_store.lock().expect("fd_store lock");
            *store.get(&tag.payload_id)
                .ok_or_else(|| TransportError::SendFailed(
                    format!("payload_id {} not found in fd_store", tag.payload_id)
                ))?
        };
        let sc_tag = super::sidechannel::FdTag {
            stream_id: tag.stream_id,
            flags: tag.flags,
            payload_size: tag.payload_size,
            payload_id: tag.payload_id,
            fd,
        };
        self.sidechannel.send_tagged_fd(&sc_tag).map_err(|e| match e {
            super::sidechannel::SideChannelError::Closed => TransportError::Closed,
            super::sidechannel::SideChannelError::WouldBlock => TransportError::WouldBlock,
            other => TransportError::SendFailed(format!("{other:?}")),
        })
    }

    fn recv_fd(&self) -> Result<FdTag, TransportError> {
        let sc_tag = self.sidechannel.recv_tagged_fd().map_err(|e| match e {
            super::sidechannel::SideChannelError::Closed => TransportError::Closed,
            super::sidechannel::SideChannelError::WouldBlock => TransportError::WouldBlock,
            other => TransportError::RecvFailed(format!("{other:?}")),
        })?;
        // Store the received fd under the sender's payload_id —
        // transmitted in the sidechannel tag alongside the fd.
        let payload_id = sc_tag.payload_id;
        self.fd_store.lock().expect("fd_store lock")
            .insert(payload_id, sc_tag.fd);
        Ok(FdTag {
            stream_id: sc_tag.stream_id,
            flags: sc_tag.flags,
            payload_size: sc_tag.payload_size,
            payload_id,
        })
    }
}

/// Production memfd operations using real `memfd_create` + `mmap`.
/// Shares an fd_store with `LinuxTransport` so both can map payload_id → fd.
#[cfg(target_os = "linux")]
pub struct LinuxMemfd {
    fd_store: std::sync::Arc<Mutex<HashMap<u32, RawFd>>>,
    next_id: Mutex<u32>,
}

#[cfg(target_os = "linux")]
impl LinuxMemfd {
    pub fn new(fd_store: std::sync::Arc<Mutex<HashMap<u32, RawFd>>>) -> Self {
        Self {
            fd_store,
            next_id: Mutex::new(0),
        }
    }
}

#[cfg(target_os = "linux")]
impl MemfdOps for LinuxMemfd {
    fn create_and_seal(&self, data: &[u8]) -> Result<(u32, [u8; 32]), MemfdError> {
        let sealed = super::memfd::write_and_seal("rti-handoff", data)
            .map_err(|e| MemfdError::CreateFailed(format!("{e:?}")))?;
        let hash = *blake3::hash(data).as_bytes();
        let fd = sealed.fd();
        let mut id = self.next_id.lock().expect("next_id lock");
        let payload_id = *id;
        *id += 1;
        // Store the fd so LinuxTransport can look it up by payload_id.
        self.fd_store.lock().expect("fd_store lock")
            .insert(payload_id, fd);
        // Leak the SealedMemfd — the fd is now owned by the fd_store.
        // It will be closed when the entry is removed or the store drops.
        std::mem::forget(sealed);
        Ok((payload_id, hash))
    }

    fn retrieve(&self, payload_id: u32) -> Option<Vec<u8>> {
        let fd = {
            let store = self.fd_store.lock().expect("fd_store lock");
            *store.get(&payload_id)?
        };
        // Get size via fstat
        let size = super::memfd::memfd_size(fd)?;
        let mapping = super::memfd::map_readonly(fd, size).ok()?;
        Some(mapping.as_slice().to_vec())
    }

    fn verify(&self, payload_id: u32, expected_hash: &[u8; 32]) -> Result<Vec<u8>, MemfdError> {
        let fd = {
            let store = self.fd_store.lock().expect("fd_store lock");
            *store.get(&payload_id)
                .ok_or_else(|| MemfdError::CreateFailed(
                    format!("payload_id {} not found", payload_id)
                ))?
        };
        let size = super::memfd::memfd_size(fd)
            .ok_or_else(|| MemfdError::CreateFailed("fstat failed".into()))?;
        let mapping = super::memfd::map_readonly(fd, size)
            .map_err(|e| MemfdError::CreateFailed(format!("{e:?}")))?;
        super::memfd::verify_content(&mapping, expected_hash)
            .map_err(|e| match e {
                super::memfd::MemfdError::VerifyFailed { expected, computed } => {
                    MemfdError::VerifyFailed { expected, computed }
                }
                other => MemfdError::CreateFailed(format!("{other:?}")),
            })?;
        Ok(mapping.as_slice().to_vec())
    }
}

#[cfg(target_os = "linux")]
impl Drop for LinuxMemfd {
    fn drop(&mut self) {
        let store = self.fd_store.lock().expect("fd_store lock");
        for &fd in store.values() {
            super::memfd::close_fd(fd);
        }
    }
}

/// Create a paired `(LinuxTransport, LinuxMemfd)` sharing one fd_store.
/// Both sides of the connection create their own pair — the transport
/// wraps their end of the sidechannel, the memfd wraps the local fd store.
#[cfg(target_os = "linux")]
pub fn linux_pair(sidechannel: SideChannel) -> (LinuxTransport, LinuxMemfd) {
    let fd_store = std::sync::Arc::new(Mutex::new(HashMap::new()));
    let transport = LinuxTransport::new(sidechannel, std::sync::Arc::clone(&fd_store));
    let memfd = LinuxMemfd::new(fd_store);
    (transport, memfd)
}
