//! Single-buffer hash implementations.

use aws_lc_rs::digest;

/// Compute a one-shot SHA-256 digest.
pub fn sha256_oneshot(data: &[u8]) -> [u8; 32] {
    let d = digest::digest(&digest::SHA256, data);
    let mut out = [0u8; 32];
    out.copy_from_slice(d.as_ref());
    out
}

/// Compute a one-shot BLAKE3 digest.
pub fn blake3_oneshot(data: &[u8]) -> [u8; 32] {
    *blake3::hash(data).as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_deterministic() {
        let d1 = sha256_oneshot(b"hello");
        let d2 = sha256_oneshot(b"hello");
        assert_eq!(d1, d2);
    }

    #[test]
    fn blake3_deterministic() {
        let d1 = blake3_oneshot(b"hello");
        let d2 = blake3_oneshot(b"hello");
        assert_eq!(d1, d2);
    }

    #[test]
    fn sha256_and_blake3_differ() {
        assert_ne!(sha256_oneshot(b"test"), blake3_oneshot(b"test"));
    }
}
