//! AES-256-GCM bulk cipher backed by aws-lc-rs.
//!
//! The cipher wraps `aws_lc_rs::aead::LessSafeKey` with explicit
//! caller-managed nonces. `LessSafeKey` is the aws-lc-rs API
//! that accepts caller-managed nonces (the "safe" `SealingKey` enforces
//! monotonic nonces internally, which prevents parallel encryption).
//!
//! # Thread safety
//!
//! `LessSafeKey` is `Send + Sync` (verified by compile-time trait
//! assertions and a 100-thread concurrent sealing test in aws-lc-rs).
//! Each `seal_in_place_separate_tag` call makes exactly one FFI call to
//! `EVP_AEAD_CTX_seal_scatter`, which allocates a stack-local
//! `GCM128_CONTEXT` per call with zero shared mutable state.
//!
//! Share via `Arc<BulkCipher>` across rayon workers.
//!
//! # Nonce construction
//!
//! The 12-byte AEAD nonce is: `[0x00; 4][8-byte counter BE]`.
//! The high 4 bytes are zero. The counter occupies the low 8 bytes
//! and is managed by the caller via `AtomicU64`. Nonce uniqueness
//! is guaranteed by the monotonic counter which never resets.
//! A new key requires a new Noise handshake.
//!
//! # Performance
//!
//! On Coffee Lake (AES-NI + AVX, no AVX-512):
//!   seal 64 KiB: ~16 µs, ~3.74 GiB/s
//!
//! On AVX-512 hardware (Ice Lake, Zen 4):
//!   seal 64 KiB: ~8 µs, ~7+ GiB/s (Intel IPsec-MB 48-block pipeline)

use aws_lc_rs::aead::{
    Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, NONCE_LEN,
};

/// AES-256-GCM authentication tag length.
pub const TAG_LEN: usize = 16;

/// Bulk encryption/decryption errors.
#[derive(Debug)]
pub enum CipherError {
    /// AEAD seal or open operation failed. Indicates nonce reuse
    /// (catastrophic), corrupted key, or tampered ciphertext.
    Aead,
    /// Output buffer is too small.
    BufferTooSmall { need: usize, have: usize },
}

impl std::fmt::Display for CipherError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Aead => write!(f, "AEAD operation failed"),
            Self::BufferTooSmall { need, have } => {
                write!(f, "buffer too small: need {need}, have {have}")
            }
        }
    }
}

impl std::error::Error for CipherError {}

/// AES-256-GCM bulk cipher.
///
/// Does NOT implement `Clone`. Share via `Arc<BulkCipher>` at the
/// call site. This avoids a redundant `Arc` layer inside the struct.
pub struct BulkCipher {
    key: LessSafeKey,
}

impl BulkCipher {
    /// Construct from a 32-byte AES-256 key.
    ///
    /// The caller should zeroize `key_bytes` after this call.
    pub fn new(key_bytes: &[u8; 32]) -> Self {
        let unbound = UnboundKey::new(&AES_256_GCM, key_bytes)
            .expect("32-byte key is always valid for AES-256-GCM");
        Self {
            key: LessSafeKey::new(unbound),
        }
    }

    /// Encrypt in-place with separate tag.
    ///
    /// - `nonce_ctr`: caller-managed counter (`AtomicU64::fetch_add` result)
    /// - `aad`: authenticated associated data (typically the 10-byte wire header)
    /// - `in_out`: plaintext on input, ciphertext on output (same length)
    ///
    /// Returns the 16-byte AEAD tag.
    pub fn seal_in_place(
        &self,
        nonce_ctr: u64,
        aad: &[u8],
        in_out: &mut [u8],
    ) -> Result<[u8; TAG_LEN], CipherError> {
        let nonce = self.build_nonce(nonce_ctr);
        let tag = self
            .key
            .seal_in_place_separate_tag(nonce, Aad::from(aad), in_out)
            .map_err(|_| CipherError::Aead)?;

        let mut tag_bytes = [0u8; TAG_LEN];
        tag_bytes.copy_from_slice(tag.as_ref());
        Ok(tag_bytes)
    }

    /// Decrypt in-place from a contiguous `[ciphertext || 16-byte tag]` buffer.
    ///
    /// On success, the first `ct_and_tag.len() - 16` bytes contain plaintext.
    /// Returns the plaintext length.
    pub fn open_in_place(
        &self,
        nonce_ctr: u64,
        aad: &[u8],
        ct_and_tag: &mut [u8],
    ) -> Result<usize, CipherError> {
        if ct_and_tag.len() < TAG_LEN {
            return Err(CipherError::BufferTooSmall {
                need: TAG_LEN,
                have: ct_and_tag.len(),
            });
        }
        let nonce = self.build_nonce(nonce_ctr);
        let plaintext = self
            .key
            .open_in_place(nonce, Aad::from(aad), ct_and_tag)
            .map_err(|_| CipherError::Aead)?;
        Ok(plaintext.len())
    }

    /// Construct the 12-byte AEAD nonce from a counter.
    ///
    /// Layout: `[0:4][counter:8 BE]`. The first 4 bytes are reserved
    /// zero. If a new key is needed, perform a new Noise handshake —
    /// do not attempt epoch rotation within the same session.
    #[allow(clippy::unused_self)]
    fn build_nonce(&self, counter: u64) -> Nonce {
        let mut nonce_bytes = [0u8; NONCE_LEN]; // 12 bytes
        nonce_bytes[4..].copy_from_slice(&counter.to_be_bytes());
        Nonce::assume_unique_for_key(nonce_bytes)
    }
}

// Compile-time trait assertions — these fail at compile time if
// LessSafeKey is not Send+Sync, which would make BulkCipher unsafe
// to share via Arc.
static_assertions::assert_impl_all!(BulkCipher: Send, Sync);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_small() {
        let cipher = BulkCipher::new(&[0x42; 32]);
        let original = b"hello world".to_vec();
        let mut buf = original.clone();

        let tag = cipher.seal_in_place(0, b"aad", &mut buf).unwrap();

        // Construct combined buffer for decryption
        let mut combined = Vec::with_capacity(buf.len() + TAG_LEN);
        combined.extend_from_slice(&buf);
        combined.extend_from_slice(&tag);

        let pt_len = cipher.open_in_place(0, b"aad", &mut combined).unwrap();
        assert_eq!(pt_len, original.len());
        assert_eq!(&combined[..pt_len], &original);
    }

    #[test]
    fn roundtrip_max_chunk() {
        let cipher = BulkCipher::new(&[0x42; 32]);
        let original = vec![0xAB; super::super::frame::MAX_CHUNK_PLAIN];
        let mut buf = original.clone();

        let tag = cipher.seal_in_place(0, b"", &mut buf).unwrap();

        let mut combined = Vec::with_capacity(buf.len() + TAG_LEN);
        combined.extend_from_slice(&buf);
        combined.extend_from_slice(&tag);

        let pt_len = cipher.open_in_place(0, b"", &mut combined).unwrap();
        assert_eq!(pt_len, original.len());
        assert_eq!(&combined[..pt_len], &original);
    }

    #[test]
    fn different_nonces_produce_different_ciphertext() {
        let cipher = BulkCipher::new(&[0x42; 32]);
        let mut buf0 = vec![0xAB; 1024];
        let mut buf1 = vec![0xAB; 1024];

        let _tag0 = cipher.seal_in_place(0, b"", &mut buf0).unwrap();
        let _tag1 = cipher.seal_in_place(1, b"", &mut buf1).unwrap();

        assert_ne!(buf0, buf1);
    }

    #[test]
    fn tampered_ciphertext_rejected() {
        let cipher = BulkCipher::new(&[0x42; 32]);
        let mut buf = vec![0xAB; 1024];

        let tag = cipher.seal_in_place(0, b"", &mut buf).unwrap();

        buf[0] ^= 0xFF; // tamper

        let mut combined = Vec::with_capacity(buf.len() + TAG_LEN);
        combined.extend_from_slice(&buf);
        combined.extend_from_slice(&tag);

        assert!(cipher.open_in_place(0, b"", &mut combined).is_err());
    }

    #[test]
    fn wrong_aad_rejected() {
        let cipher = BulkCipher::new(&[0x42; 32]);
        let mut buf = vec![0xAB; 1024];

        let tag = cipher.seal_in_place(0, b"correct", &mut buf).unwrap();

        let mut combined = Vec::with_capacity(buf.len() + TAG_LEN);
        combined.extend_from_slice(&buf);
        combined.extend_from_slice(&tag);

        assert!(cipher.open_in_place(0, b"wrong", &mut combined).is_err());
    }

    #[test]
    fn wrong_nonce_rejected() {
        let cipher = BulkCipher::new(&[0x42; 32]);
        let mut buf = vec![0xAB; 1024];

        let tag = cipher.seal_in_place(0, b"", &mut buf).unwrap();

        let mut combined = Vec::with_capacity(buf.len() + TAG_LEN);
        combined.extend_from_slice(&buf);
        combined.extend_from_slice(&tag);

        // Decrypt with wrong nonce
        assert!(cipher.open_in_place(1, b"", &mut combined).is_err());
    }

    #[test]
    fn different_keys_rejected() {
        let cipher0 = BulkCipher::new(&[0x42; 32]);
        let cipher1 = BulkCipher::new(&[0x43; 32]);
        let mut buf = vec![0xAB; 1024];

        let tag = cipher0.seal_in_place(0, b"", &mut buf).unwrap();

        let mut combined = Vec::with_capacity(buf.len() + TAG_LEN);
        combined.extend_from_slice(&buf);
        combined.extend_from_slice(&tag);

        assert!(cipher1.open_in_place(0, b"", &mut combined).is_err());
    }

    #[test]
    fn buffer_too_small_error() {
        let cipher = BulkCipher::new(&[0x42; 32]);
        let mut tiny = [0u8; 15]; // less than TAG_LEN
        assert!(cipher.open_in_place(0, b"", &mut tiny).is_err());
    }
}
