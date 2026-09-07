//! Content hashing helpers.

/// BLAKE3 hash of `bytes`, lowercase hex.
///
/// The one spelling of this. `rekindle-channel` and `rekindle-sync`
/// each carried a byte-identical copy, on both sides of the same
/// content-hash contract: the channel track computes the hash that goes
/// into a gossip announcement, the sync track recomputes it to verify
/// fetched ciphertext. Two copies of the function that has to agree for
/// that check to mean anything.
///
/// BLAKE3 is deliberately not one of the gated crypto crates (see
/// `CRYPTO_CRATES` in `xtask`) — it is a hash, not a secret operation,
/// so this belongs in utils rather than behind the `rekindle-secrets`
/// Tier-2 boundary.
#[must_use]
pub fn blake3_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::blake3_hex;

    #[test]
    fn known_answer_matches_the_blake3_spec() {
        // BLAKE3 of the empty input — official test vector, so this
        // pins the encoding (lowercase hex, untruncated) and not just
        // "whatever the library returned today".
        assert_eq!(
            blake3_hex(b""),
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
    }

    #[test]
    fn output_is_64_lowercase_hex_chars() {
        let hash = blake3_hex(b"rekindle");
        assert_eq!(hash.len(), 64);
        assert!(hash
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn distinct_inputs_hash_differently() {
        assert_ne!(blake3_hex(b"a"), blake3_hex(b"b"));
    }
}
