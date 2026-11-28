//! The spec Double Ratchet (Signal, "The Double Ratchet Algorithm",
//! revision 4, 2025-11-04) for the pure session API (plan E2.2).
//!
//! It is §3's state machine (§3.2 state, §3.3 initialization, §3.4
//! encrypt, §3.5 decrypt) with §7.2's recommended functions: `KDF_RK` is
//! HKDF-SHA256 (salt the root key, info `DR_ROOT`), `KDF_CK` is
//! HMAC-SHA256 with the constants `0x01` (message key) and `0x02` (next
//! chain key), and `ENCRYPT` is §7.2's concrete recipe: HKDF-SHA256 of
//! the message key (info `DR_MSG`) into an AES-256 key, an HMAC-SHA256 key
//! and an IV; AES-256-CBC with PKCS#7; HMAC-SHA256 over `AD ‖ ciphertext`
//! truncated to 128 bits (§8.6).
//!
//! Skipped message keys are bounded three ways: a message may run at most
//! [`MAX_SKIP`] ahead of its chain (§3.2, §3.5), at most
//! [`MAX_SKIPPED_KEYS_PER_CHAIN`] skipped keys are kept per receiving
//! chain (oldest dropped first), and only the newest
//! [`MAX_RECEIVING_CHAINS_KEPT`] receiving chains keep theirs (oldest
//! dropped first: §8.4's deletion "by counting ... DH ratchet steps").
//! §8.4 also asks for deletion "after an appropriate interval", and a
//! conversation whose peer never answers takes no DH ratchet step, so a
//! skipped key is also deleted [`SKIPPED_KEY_MAX_AGE_MS`] after it was
//! stored. The values are the bounds Signal's deployed clients use.

mod codec;
mod kdf;
mod state;
#[cfg(test)]
mod tests;

pub(crate) use codec::{corrupt, Reader};
pub(crate) use state::SessionState;

/// Skipped message keys kept per receiving chain, oldest dropped first
/// (DR rev. 4 §8.4).
pub const MAX_SKIPPED_KEYS_PER_CHAIN: usize = 2000;

/// Receiving chains whose skipped keys a session keeps, oldest dropped
/// first (DR rev. 4 §8.4, deletion by counting DH ratchet steps).
pub const MAX_RECEIVING_CHAINS_KEPT: usize = 5;

/// The furthest a message may run ahead of its chain: the spec's
/// `MAX_SKIP` (DR rev. 4 §3.2, §3.5).
pub const MAX_SKIP: u32 = 25_000;

/// How long a skipped message key is kept: the delivery horizon (plan
/// D24), the longest a message may take to arrive, after which its key
/// can only serve an attacker who recorded it (DR rev. 4 §8.4).
pub const SKIPPED_KEY_MAX_AGE_MS: u64 = crate::horizon::DELIVERY_HORIZON.as_secs() * 1_000;

/// Length of [`Header::to_bytes`].
const HEADER_BYTES: usize = 40;

/// A message header (§3.1 `HEADER`): the sender's ratchet public key, the length
/// of its previous sending chain and the message's number in this one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub dh: [u8; 32],
    pub pn: u32,
    pub n: u32,
}

impl Header {
    /// The header as the AD binds it: `dh ‖ u32be(pn) ‖ u32be(n)`. Fixed
    /// length and last in the AD, so `AD ‖ header` parses one way (§3.4
    /// `CONCAT`).
    #[must_use]
    pub fn to_bytes(self) -> [u8; HEADER_BYTES] {
        let mut out = [0u8; HEADER_BYTES];
        out[..32].copy_from_slice(&self.dh);
        out[32..36].copy_from_slice(&self.pn.to_be_bytes());
        out[36..].copy_from_slice(&self.n.to_be_bytes());
        out
    }
}

/// One ratchet message: its header and the CBC ciphertext with its
/// truncated HMAC tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RatchetMessage {
    pub header: Header,
    pub ciphertext: Vec<u8>,
}
