//! Noise IK encrypted IPC transport.
//!
//! Provides forward-secret, mutually-authenticated encryption for all
//! IPC traffic using the Noise Protocol Framework (IK pattern) via `snow`.
//!
//! # Module layout
//!
//! - `writer`    — `NoiseWriter`: encrypt half, owns send nonce + enc_buf
//! - `reader`    — `NoiseReader`: decrypt half, owns recv nonce + dec_buf + read_buf
//! - `handshake` — `server_handshake`, `client_handshake` → `NoiseTransport`
//!
//! # Architecture
//!
//! `NoiseTransport` is the result of a completed handshake. It contains
//! a `NoiseWriter` and a `NoiseReader` that can be split for independent
//! use on separate tasks. Both hold `Arc<snow::TransportState>` internally
//! but use independent nonce counters and independent buffers, so no
//! `&mut self` conflict exists between concurrent read and write.

mod handshake;
pub mod reader;
pub mod writer;

#[cfg(test)]
mod tests;

pub use handshake::{server_handshake, client_handshake};
pub use reader::NoiseReader;
pub use writer::NoiseWriter;

use crate::ipc::transport::PeerCredentials;

/// Maximum plaintext per Noise transport message: 65535 - 16 (AEAD tag) = 65519.
pub(crate) const MAX_NOISE_PLAINTEXT: usize = 65535 - 16;

/// Handshake timeout to prevent DoS via slow handshake.
pub(crate) const HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Build the Noise prologue from peer credentials.
///
/// Format: `REKINDLE-IPC-v1:{lower_pid}:{lower_uid}:{higher_pid}:{higher_uid}`
///
/// Canonical ordering (lower PID first) ensures both sides produce identical
/// bytes regardless of who initiates.
pub(crate) fn build_prologue(local: &PeerCredentials, remote: &PeerCredentials) -> Vec<u8> {
    let (first, second) = if local.pid <= remote.pid {
        (local, remote)
    } else {
        (remote, local)
    };
    format!(
        "REKINDLE-IPC-v1:{}:{}:{}:{}",
        first.pid, first.uid, second.pid, second.uid
    )
    .into_bytes()
}

/// Result of a completed Noise IK handshake.
///
/// Contains the writer half, reader half, and the handshake hash for
/// bulk cipher derivation. Split the writer and reader onto separate
/// tasks for concurrent I/O without head-of-line blocking.
pub struct NoiseTransport {
    pub writer: NoiseWriter,
    pub reader: NoiseReader,
    handshake_hash: Option<zeroize::Zeroizing<[u8; 32]>>,
}

impl NoiseTransport {
    /// Take the handshake hash for bulk cipher key derivation.
    ///
    /// Returns `Some([u8; 32])` on the first call, `None` on all
    /// subsequent calls. The internal copy is zeroized on take.
    pub fn take_handshake_hash(&mut self) -> Option<[u8; 32]> {
        self.handshake_hash.take().map(|z| *z)
    }

    /// The remote party's static public key.
    pub fn remote_static(&self) -> Option<&[u8]> {
        self.reader.remote_static()
    }
}
