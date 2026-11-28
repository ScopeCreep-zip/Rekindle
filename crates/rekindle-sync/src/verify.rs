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

pub use rekindle_utils::blake3_hex;

#[cfg(test)]
#[path = "verify_tests.rs"]
mod tests;
