//! Bulk cipher: algorithm-polymorphic AEAD for the transport pipeline.
//!
//! Wraps `rekindle_aead::BulkAead` (trait object) with a u64-nonce
//! convenience API. The underlying algorithm is selected at construction
//! time — `AesGcmKey` (default) or `Aegis128LKey` (feature `aegis`).
//!
//! Callers pass `u64` nonce counters from `NonceCounter::next()`. The
//! cipher builds the algorithm-specific nonce bytes internally.
//!
//! # Thread safety
//!
//! `BulkAead` requires `Send + Sync`. Both `AesGcmKey` and `Aegis128LKey`
//! are stateless after construction (key schedule in CPU registers or
//! pre-expanded tables). Share via `Arc<BulkCipher>` across rayon workers.
//!
//! # Performance
//!
//! On Coffee Lake (AES-NI + AVX, no AVX-512):
//!   AES-256-GCM seal 64 KiB: ~16 µs, ~3.74 GiB/s
//!   AEGIS-128L seal 64 KiB: ~8 µs, ~6–8 GiB/s (feature `aegis`)

use rekindle_aead::{AeadAlgorithm, AeadError, BulkAead};
use rekindle_aead::aes_gcm::AesGcmKey;

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
    /// Requested AEAD algorithm is not available (feature not compiled).
    UnsupportedAlgorithm(AeadAlgorithm),
}

impl std::fmt::Display for CipherError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Aead => write!(f, "AEAD operation failed"),
            Self::BufferTooSmall { need, have } => {
                write!(f, "buffer too small: need {need}, have {have}")
            }
            Self::UnsupportedAlgorithm(algo) => {
                write!(f, "unsupported AEAD algorithm: {algo:?} (feature not compiled)")
            }
        }
    }
}

impl std::error::Error for CipherError {}

impl From<AeadError> for CipherError {
    fn from(e: AeadError) -> Self {
        match e {
            AeadError::AuthFailed | AeadError::Init => Self::Aead,
            AeadError::BufferSize { need, have } => Self::BufferTooSmall { need, have },
        }
    }
}

/// Algorithm-polymorphic bulk cipher.
///
/// Does NOT implement `Clone`. Share via `Arc<BulkCipher>` at the
/// call site. This avoids a redundant `Arc` layer inside the struct.
pub struct BulkCipher {
    inner: Box<dyn BulkAead>,
}

impl BulkCipher {
    /// Construct AES-256-GCM from a 32-byte key (default algorithm).
    ///
    /// The caller should zeroize `key_bytes` after this call.
    pub fn new(key_bytes: &[u8; 32]) -> Self {
        let key = AesGcmKey::new(key_bytes)
            .expect("32-byte key is always valid for AES-256-GCM");
        Self { inner: Box::new(key) }
    }

    /// Construct with a specific algorithm.
    ///
    /// Returns `Err(CipherError::UnsupportedAlgorithm)` if the requested
    /// algorithm requires a feature flag that is not compiled in.
    ///
    /// `key_bytes` is 32 bytes for AES-256-GCM. For AEGIS-128L with
    /// 32-byte HKDF output, the first 16 bytes are used via `from_32`.
    #[allow(clippy::unnecessary_wraps)] // Returns Err when aegis feature is disabled
    pub fn with_algorithm(algorithm: AeadAlgorithm, key_bytes: &[u8; 32]) -> Result<Self, CipherError> {
        match algorithm {
            AeadAlgorithm::Aes256Gcm => Ok(Self::new(key_bytes)),
            #[cfg(feature = "aegis")]
            AeadAlgorithm::Aegis128L => {
                let key = rekindle_aead::aegis128l::Aegis128LKey::from_32(key_bytes);
                Ok(Self { inner: Box::new(key) })
            }
            #[cfg(not(feature = "aegis"))]
            AeadAlgorithm::Aegis128L => {
                Err(CipherError::UnsupportedAlgorithm(algorithm))
            }
        }
    }

    /// The algorithm this cipher uses.
    pub fn algorithm(&self) -> AeadAlgorithm {
        self.inner.algorithm()
    }

    /// Encrypt in-place with separate tag.
    ///
    /// - `nonce_ctr`: caller-managed counter (`NonceCounter::next()` result)
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
        let nonce = self.inner.build_nonce(nonce_ctr);
        Ok(self.inner.seal_in_place(&nonce, aad, in_out)?)
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
        let nonce = self.inner.build_nonce(nonce_ctr);
        Ok(self.inner.open_in_place(&nonce, aad, ct_and_tag)?)
    }

    /// Decrypt with detached tag — zero allocation, zero memmove.
    ///
    /// `ciphertext.len()` must equal `plaintext_out.len()`.
    /// `tag` must be at least `TAG_LEN` (16) bytes.
    pub fn open_separate(
        &self,
        nonce_ctr: u64,
        aad: &[u8],
        ciphertext: &[u8],
        tag: &[u8],
        plaintext_out: &mut [u8],
    ) -> Result<(), CipherError> {
        if ciphertext.len() != plaintext_out.len() {
            return Err(CipherError::BufferTooSmall {
                need: ciphertext.len(),
                have: plaintext_out.len(),
            });
        }
        if tag.len() < TAG_LEN {
            return Err(CipherError::BufferTooSmall {
                need: TAG_LEN,
                have: tag.len(),
            });
        }
        let nonce = self.inner.build_nonce(nonce_ctr);
        self.inner
            .open_detached(&nonce, aad, ciphertext, tag, plaintext_out)
            .map_err(CipherError::from)
    }
}

// Compile-time trait assertions — BulkAead requires Send + Sync,
// and Box<dyn BulkAead> inherits that.
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

    #[test]
    fn open_separate_roundtrip() {
        let cipher = BulkCipher::new(&[0x42; 32]);
        let original = vec![0xAB; super::super::frame::MAX_CHUNK_PLAIN];
        let mut ct = original.clone();
        let tag = cipher.seal_in_place(0, b"aad", &mut ct).unwrap();
        let mut pt = vec![0u8; original.len()];
        cipher.open_separate(0, b"aad", &ct, &tag, &mut pt).unwrap();
        assert_eq!(pt, original);
    }

    #[test]
    fn open_separate_wrong_tag_rejected() {
        let cipher = BulkCipher::new(&[0x42; 32]);
        let mut ct = vec![0xAB; 1024];
        let tag = cipher.seal_in_place(0, b"", &mut ct).unwrap();
        ct[0] ^= 0xFF;
        let mut pt = vec![0u8; 1024];
        assert!(cipher.open_separate(0, b"", &ct, &tag, &mut pt).is_err());
    }

    #[test]
    fn open_separate_wrong_nonce_rejected() {
        let cipher = BulkCipher::new(&[0x42; 32]);
        let mut ct = vec![0xAB; 1024];
        let tag = cipher.seal_in_place(0, b"", &mut ct).unwrap();
        let mut pt = vec![0u8; 1024];
        assert!(cipher.open_separate(1, b"", &ct, &tag, &mut pt).is_err());
    }

    #[test]
    fn open_separate_size_mismatch_rejected() {
        let cipher = BulkCipher::new(&[0x42; 32]);
        let ct = vec![0xAB; 1024];
        let tag = [0u8; TAG_LEN];
        let mut pt = vec![0u8; 512]; // wrong size
        assert!(cipher.open_separate(0, b"", &ct, &tag, &mut pt).is_err());
    }
}
