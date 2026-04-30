//! Content hash verification for the Chiral Network delivery model.
//!
//! Gossip notifications carry a `content_hash` (blake3 of ciphertext) so
//! recipients can verify that the DHT-fetched ciphertext matches what the
//! sender wrote. This prevents substitution attacks on the DHT storage layer.

/// Verify that the blake3 hash of fetched ciphertext matches the expected hash
/// from the gossip notification.
pub fn verify_content_hash(ciphertext: &[u8], expected_hash: &str) -> bool {
    let computed = blake3::hash(ciphertext);
    computed.to_hex().as_str() == expected_hash
}

/// Compute the blake3 hex hash of a byte slice.
pub fn blake3_hex(data: &[u8]) -> String {
    blake3::hash(data).to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rekindle_protocol::dht::community::envelope::CommunityEnvelope;

    #[test]
    fn accepts_matching_ciphertext() {
        let ciphertext = b"encrypted message payload from SMPL write";
        let hash = blake3_hex(ciphertext);
        assert!(verify_content_hash(ciphertext, &hash));
    }

    #[test]
    fn rejects_tampered_ciphertext() {
        let original = b"encrypted message payload from SMPL write";
        let hash = blake3_hex(original);
        let tampered = b"tampered message payload from evil node";
        assert!(!verify_content_hash(tampered, &hash));
    }

    #[test]
    fn rejects_single_bit_flip() {
        let original = b"encrypted message payload";
        let hash = blake3_hex(original);
        let mut flipped = original.to_vec();
        flipped[0] ^= 1;
        assert!(!verify_content_hash(&flipped, &hash));
    }

    #[test]
    fn empty_ciphertext_has_deterministic_hash() {
        let hash1 = blake3_hex(b"");
        let hash2 = blake3_hex(b"");
        assert_eq!(hash1, hash2);
        assert!(verify_content_hash(b"", &hash1));
    }

    #[test]
    fn rejects_tampered_ciphertext_for_message_notification_hash() {
        let ciphertext = b"ciphertext-from-sender";
        let notification = CommunityEnvelope::MessageNotification {
            channel_id: "ch_01".into(),
            message_id: "msg_01".into(),
            author_pseudonym: "author_01".into(),
            subkey_index: 4,
            lamport_ts: 99,
            sequence: 7,
            content_hash: blake3_hex(ciphertext),
            timestamp: 1_700_000_000,
        };

        let tampered = b"ciphertext-from-attacker";
        let expected_hash = match notification {
            CommunityEnvelope::MessageNotification { content_hash, .. } => content_hash,
            _ => unreachable!(),
        };

        assert!(!verify_content_hash(tampered, &expected_hash));
    }
}
