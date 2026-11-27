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
#[path = "hash_tests.rs"]
mod tests;
