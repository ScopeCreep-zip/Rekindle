//! The spec Double Ratchet (Signal, "The Double Ratchet Algorithm",
//! rev. 1, 2016-11-20) for the pure session API (plan E2.2).
//!
//! It is §3.5's state machine with §5.2's recommended functions:
//! `KDF_RK` is HKDF-SHA256 (salt the root key, info `DR_ROOT`), `KDF_CK`
//! is HMAC-SHA256 with the constants `0x01` (message key) and `0x02` (next
//! chain key), and each message key expands under `DR_MSG` into an
//! AES-256-GCM key and nonce. An AEAD is allowed in place of §5.2's
//! CBC+HMAC because every message key encrypts exactly one message.
//!
//! Skipped message keys are bounded as libsignal bounds them (commit
//! `1738ead8`, `consts.rs`): at most [`MAX_MESSAGE_KEYS`] per receiving
//! chain (oldest dropped first), at most [`MAX_RECEIVER_CHAINS`] receiving
//! chains (oldest dropped first), and a forward jump of at most
//! [`MAX_FORWARD_JUMPS`] within a chain.
//!
//! `RatchetState` in [`super::ratchet`] is the non-spec ratchet the
//! session manager still runs; plan E2.3 deletes it.

mod codec;
mod kdf;
mod state;
#[cfg(test)]
mod tests;

pub(crate) use codec::{corrupt, Reader};
pub(crate) use state::SessionState;

/// Skipped message keys kept per receiving chain (libsignal
/// `MAX_MESSAGE_KEYS`).
pub const MAX_MESSAGE_KEYS: usize = 2000;

/// Receiving chains kept per session (libsignal `MAX_RECEIVER_CHAINS`).
pub const MAX_RECEIVER_CHAINS: usize = 5;

/// The furthest a message may run ahead of its chain (libsignal
/// `MAX_FORWARD_JUMPS`; the spec's `MAX_SKIP`).
pub const MAX_FORWARD_JUMPS: u32 = 25_000;

/// Length of [`Header::to_bytes`].
const HEADER_BYTES: usize = 40;

/// A message header (§3.2): the sender's ratchet public key, the length
/// of its previous sending chain and the message's number in this one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub dh: [u8; 32],
    pub pn: u32,
    pub n: u32,
}

impl Header {
    /// The header as the AD binds it: `dh ‖ u32be(pn) ‖ u32be(n)`. Fixed
    /// length and last in the AD, so `AD ‖ header` parses one way (§3.4).
    #[must_use]
    pub fn to_bytes(self) -> [u8; HEADER_BYTES] {
        let mut out = [0u8; HEADER_BYTES];
        out[..32].copy_from_slice(&self.dh);
        out[32..36].copy_from_slice(&self.pn.to_be_bytes());
        out[36..].copy_from_slice(&self.n.to_be_bytes());
        out
    }
}

/// One ratchet message: its header and the AEAD ciphertext with its tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RatchetMessage {
    pub header: Header,
    pub ciphertext: Vec<u8>,
}
