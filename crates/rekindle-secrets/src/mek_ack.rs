//! The key-confirmation tag of a `MekTransferAck` (review S7m part 1,
//! research batch 11 (2), as RFC 9420 §12.4.3's `confirmation_tag`): the
//! joiner proves it unwrapped exactly the key of generation `G`, not just
//! that a packet arrived, by a MAC keyed from that key:
//!
//! `tag = BLAKE3-keyed(derive_key(MEK_ACK, MEK_G), community ‖ scope ‖ G ‖
//! transfer_id ‖ inviter ‖ joiner)`, every variable field length-prefixed.
//!
//! The inviter recomputes it under its own copy of the key. No signature:
//! the ack travels inside the authenticated app_call and only the inviter
//! consumes it, so the joiner keeps deniability.

use subtle::ConstantTimeEq;

use rekindle_types::domains;

/// What one ack confirms.
#[derive(Debug, Clone, Copy)]
pub struct MekAckContext<'a> {
    pub community_id: &'a str,
    /// The key scope's wire form (empty for the community key).
    pub scope: &'a str,
    pub generation: u64,
    /// Random per push and kept across its retries; it correlates, it does
    /// not settle (any valid tag for the generation does).
    pub transfer_id: &'a [u8],
    /// The inviter's (sender's) pseudonym, hex.
    pub inviter: &'a str,
    /// The joiner's (receiver's) pseudonym, hex.
    pub joiner: &'a str,
}

impl MekAckContext<'_> {
    fn message(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let parts: [&[u8]; 5] = [
            self.community_id.as_bytes(),
            self.scope.as_bytes(),
            self.transfer_id,
            self.inviter.as_bytes(),
            self.joiner.as_bytes(),
        ];
        for (i, part) in parts.iter().enumerate() {
            out.extend_from_slice(&u32::try_from(part.len()).unwrap_or(u32::MAX).to_be_bytes());
            out.extend_from_slice(part);
            if i == 1 {
                out.extend_from_slice(&self.generation.to_be_bytes());
            }
        }
        out
    }
}

/// The tag the joiner answers with after unwrapping `mek` (the key's 32
/// bytes).
#[must_use]
pub fn mek_ack_tag(mek: &[u8; 32], ctx: &MekAckContext<'_>) -> [u8; 32] {
    let key = zeroize::Zeroizing::new(blake3::derive_key(domains::MEK_ACK, mek));
    *blake3::keyed_hash(&key, &ctx.message()).as_bytes()
}

/// Whether `tag` confirms `mek` for `ctx`, in constant time.
#[must_use]
pub fn verify_mek_ack_tag(mek: &[u8; 32], ctx: &MekAckContext<'_>, tag: &[u8]) -> bool {
    tag.len() == 32 && bool::from(mek_ack_tag(mek, ctx).as_slice().ct_eq(tag))
}

#[cfg(test)]
#[path = "mek_ack_tests.rs"]
mod tests;
