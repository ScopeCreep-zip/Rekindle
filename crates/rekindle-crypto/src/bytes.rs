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
        CryptoError::InvalidKey(format!("{what}: expected 32 bytes, got {}", bytes.len()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_32_parses() {
        assert_eq!(to_32(&[7u8; 32], "k").unwrap(), [7u8; 32]);
    }

    #[test]
    fn short_slice_errors_with_label_and_len() {
        let err = to_32(&[0u8; 31], "signed prekey").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("signed prekey"), "{msg}");
        assert!(msg.contains("31"), "{msg}");
    }

    #[test]
    fn long_slice_errors_instead_of_truncating() {
        // The old daemon-track helper took the first 32 bytes of longer
        // input; key material silently losing bytes is never right.
        let err = to_32(&[0u8; 40], "identity key").unwrap_err();
        assert!(err.to_string().contains("40"));
    }
}
