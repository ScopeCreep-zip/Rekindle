//! The replay id of a pre-session frame (plan E2.3) and the id of a
//! ratchet message (plan E2.5): the receiver keeps
//! `(sender, nonce)` for the freshness window under this id, a BLAKE3
//! derive-key hash under its own context (`docs/security/crypto-primitives.md`
//! §7: BLAKE3 for domain-separated identifiers).

use rekindle_types::domains;

/// The replay id of the frame `sender` sealed under `nonce`.
#[must_use]
pub fn replay_id(sender: &[u8; 32], nonce: &[u8; 16]) -> [u8; 32] {
    let mut input = [0u8; 48];
    input[..32].copy_from_slice(sender);
    input[32..].copy_from_slice(nonce);
    blake3::derive_key(domains::DM_REPLAY_ID, &input)
}

/// The id of the ratchet message whose header is `(dh, n)`: unique per
/// message per session, and computable by the receiver of a message it
/// could not open, so a `RetryRequest` can name it (plan E2.5, D24).
#[must_use]
pub fn ratchet_message_id(dh: &[u8; 32], n: u32) -> [u8; 32] {
    let mut input = [0u8; 36];
    input[..32].copy_from_slice(dh);
    input[32..].copy_from_slice(&n.to_be_bytes());
    blake3::derive_key(domains::DM_RATCHET_MESSAGE_ID, &input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_message_id_binds_the_ratchet_key_and_the_counter() {
        let id = ratchet_message_id(&[1; 32], 7);
        assert_eq!(id, ratchet_message_id(&[1; 32], 7));
        assert_ne!(id, ratchet_message_id(&[2; 32], 7));
        assert_ne!(id, ratchet_message_id(&[1; 32], 8));
        assert_ne!(id, replay_id(&[1; 32], &[0; 16]));
    }

    #[test]
    fn the_id_binds_the_sender_and_the_nonce() {
        let id = replay_id(&[1; 32], &[2; 16]);
        assert_eq!(id, replay_id(&[1; 32], &[2; 16]));
        assert_ne!(id, replay_id(&[3; 32], &[2; 16]));
        assert_ne!(id, replay_id(&[1; 32], &[4; 16]));
        // Under its own context, not a bare hash of the same bytes.
        let mut input = [1u8; 48];
        input[32..].copy_from_slice(&[2; 16]);
        assert_ne!(id, *blake3::hash(&input).as_bytes());
    }
}
