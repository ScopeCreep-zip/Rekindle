//! The bytes a 1:1 `MessageEnvelope` signature covers.
//!
//! `MSG_ENVELOPE_V2 ‖ recipient ‖ timestamp LE ‖ u32 LE len(nonce) ‖ nonce
//! ‖ u32 LE len(payload) ‖ payload`.
//!
//! The domain label keeps the signature from being valid in any other
//! Rekindle protocol; the recipient's identity key keeps a captured
//! envelope from being replayed to anyone else (an `Unfriended` sent to
//! Bob is meaningless to Carol); the length prefixes make the nonce /
//! payload boundary unambiguous.

use rekindle_types::domains::MSG_ENVELOPE_V2;

/// Signed bytes for an envelope from the signer to `recipient`.
#[must_use]
pub fn envelope_signing_bytes(
    recipient: &[u8; 32],
    timestamp: u64,
    nonce: &[u8],
    payload: &[u8],
) -> Vec<u8> {
    let mut bytes =
        Vec::with_capacity(MSG_ENVELOPE_V2.len() + 32 + 8 + 4 + nonce.len() + 4 + payload.len());
    bytes.extend_from_slice(MSG_ENVELOPE_V2.as_bytes());
    bytes.extend_from_slice(recipient);
    bytes.extend_from_slice(&timestamp.to_le_bytes());
    push_len_prefixed(&mut bytes, nonce);
    push_len_prefixed(&mut bytes, payload);
    bytes
}

fn push_len_prefixed(bytes: &mut Vec<u8>, field: &[u8]) {
    // Envelopes are bounded far below 4 GiB by Veilid's message size, so
    // the length always fits; saturating keeps the encoding total.
    let len = u32::try_from(field.len()).unwrap_or(u32::MAX);
    bytes.extend_from_slice(&len.to_le_bytes());
    bytes.extend_from_slice(field);
}
