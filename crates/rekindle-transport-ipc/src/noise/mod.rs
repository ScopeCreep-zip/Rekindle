//! Noise IK encrypted IPC transport.
//!
//! Provides forward-secret, mutually-authenticated encryption for all
//! IPC traffic using the Noise Protocol Framework (IK pattern).
//!
//! Uses `StatelessTransportState` which takes `&self` for both encrypt
//! and decrypt with explicit nonces — writer and reader run on separate
//! tasks without `&mut self` conflicts.

pub mod resolver;
pub mod handshake;
pub mod reader;
pub mod writer;
pub mod keys;

pub use handshake::{client_handshake, server_handshake};
pub use reader::NoiseReader;
pub use writer::NoiseWriter;

use crate::socket::PeerCredentials;

/// Maximum plaintext per Noise transport message: 65535 - 16 (AEAD tag) = 65519.
pub(crate) const MAX_NOISE_PLAINTEXT: usize = 65535 - 16;

/// Per-chunk wire overhead on top of the application payload.
///
/// Two distinct sources:
/// - Application framing: APP tag (1 byte) + sequence number (8 bytes) = 9 bytes
///   Added by `tag_application_frame` before the payload reaches snow.
/// - Snow AEAD: authentication tag (16 bytes) = snow's TAGLEN constant.
///   Added by `StatelessTransportState::write_message`.
///
/// Total: 9 + 16 = 25 bytes per chunk.
///
/// The 4-byte chunk count header is a separate `write_frame`/`read_frame` call
/// and is NOT part of the per-chunk limit.
///
/// Used to derive the wire-level max for NoiseReader and NoiseWriter from the
/// config's application-level `max_frame_size`. Both reader and writer accept
/// frames up to `config.max_frame_size + WIRE_OVERHEAD_PER_CHUNK` bytes.
pub(crate) const WIRE_OVERHEAD_PER_CHUNK: u32 = 9 + 16; // app framing (9) + AEAD tag (16)

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
/// Contains writer half, reader half, and handshake hash for bulk cipher
/// derivation. Split the writer and reader onto separate tasks.
pub struct NoiseTransport {
    pub writer: NoiseWriter,
    pub reader: NoiseReader,
    handshake_hash: Option<zeroize::Zeroizing<[u8; 32]>>,
}

impl NoiseTransport {
    /// Take the handshake hash for bulk cipher key derivation.
    ///
    /// Returns `Some` on first call, `None` thereafter. Internal copy zeroized.
    pub fn take_handshake_hash(&mut self) -> Option<[u8; 32]> {
        self.handshake_hash.take().map(|z| *z)
    }

    /// The remote party's static public key.
    pub fn remote_static(&self) -> Option<&[u8]> {
        self.reader.remote_static()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prologue_canonical_ordering() {
        let a = PeerCredentials { pid: 100, uid: 1000 };
        let b = PeerCredentials { pid: 200, uid: 1000 };
        assert_eq!(build_prologue(&a, &b), build_prologue(&b, &a));
        let p = String::from_utf8(build_prologue(&a, &b)).unwrap();
        assert_eq!(p, "REKINDLE-IPC-v1:100:1000:200:1000");
    }
}
