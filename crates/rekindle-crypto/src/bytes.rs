//! Byte-slice parsing helpers shared across the crypto modules.

use crate::error::CryptoError;

/// Parse a slice that must be EXACTLY 32 bytes into a fixed array,
/// naming the key material in the error for diagnosability.
///
/// THE implementation for the `[u8] → [u8; 32]` checks that were
/// previously re-spelled in the ratchet core (exact-length) and the
/// daemon track's session manager (which took a 32-byte prefix —
/// silently truncating longer input and PANICKING on shorter). Every
/// production caller passes exactly 32 bytes, so the strict form is
/// the correct one: malformed input now errors instead of truncating
/// or panicking.
pub fn to_32(bytes: &[u8], what: &str) -> Result<[u8; 32], CryptoError> {
    <[u8; 32]>::try_from(bytes).map_err(|_| {
        CryptoError::invalid_key(format!("{what}: expected 32 bytes, got {}", bytes.len()))
    })
}

#[cfg(test)]
#[path = "bytes_tests.rs"]
mod tests;
