//! Bulk cipher: algorithm-polymorphic AEAD for the transport pipeline.
//!
//! Wraps `rekindle_aead::BulkAead` (trait object). The underlying
//! algorithm is selected at construction: AesGcmKey (default) or
//! Aegis128LKey (feature `aegis`).
//!
//! Callers pass u64 nonce counters from `NonceCounter::next()`. The
//! cipher builds algorithm-specific nonce bytes internally.
//!
//! Thread safety: BulkAead requires Send + Sync. Both AesGcmKey and
//! Aegis128LKey are stateless after construction. Share via Arc<BulkCipher>.

use rekindle_aead::{AeadAlgorithm, AeadError, BulkAead};
use rekindle_aead::aes_gcm::AesGcmKey;

/// AES-256-GCM authentication tag length.
pub const CIPHER_TAG_LEN: usize = 16;

/// Bulk encryption/decryption errors.
#[derive(Debug)]
pub enum CipherError {
    /// AEAD operation failed (nonce reuse, corrupted key, tampered data).
    Aead,
    /// Output buffer too small.
    BufferTooSmall { need: usize, have: usize },
    /// Requested algorithm not compiled in.
    UnsupportedAlgorithm(AeadAlgorithm),
}

impl std::fmt::Display for CipherError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Aead => write!(f, "AEAD operation failed"),
            Self::BufferTooSmall { need, have } => {
                write!(f, "buffer too small: need {need}, have {have}")
            }
            Self::UnsupportedAlgorithm(a) => {
                write!(f, "unsupported AEAD: {a:?} (feature not compiled)")
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

/// Algorithm-polymorphic bulk cipher. Does NOT implement Clone.
/// Share via `Arc<BulkCipher>`.
pub struct BulkCipher {
    inner: Box<dyn BulkAead>,
}

impl BulkCipher {
    /// Construct AES-256-GCM from a 32-byte key (default).
    pub fn new(key_bytes: &[u8; 32]) -> Self {
        let key = AesGcmKey::new(key_bytes).expect("32-byte key valid for AES-256-GCM");
        Self { inner: Box::new(key) }
    }

    /// Construct with a specific algorithm.
    #[allow(clippy::unnecessary_wraps)]
    pub fn with_algorithm(
        algorithm: AeadAlgorithm,
        key_bytes: &[u8; 32],
    ) -> Result<Self, CipherError> {
        match algorithm {
            AeadAlgorithm::Aes256Gcm => Ok(Self::new(key_bytes)),
            #[cfg(feature = "aegis")]
            AeadAlgorithm::Aegis128L => {
                let key = rekindle_aead::aegis128l::Aegis128LKey::from_32(key_bytes);
                Ok(Self { inner: Box::new(key) })
            }
            #[cfg(not(feature = "aegis"))]
            AeadAlgorithm::Aegis128L => Err(CipherError::UnsupportedAlgorithm(algorithm)),
        }
    }

    pub fn algorithm(&self) -> AeadAlgorithm {
        self.inner.algorithm()
    }

    /// Encrypt in-place with separate tag.
    pub fn seal_in_place(
        &self,
        nonce_ctr: u64,
        aad: &[u8],
        in_out: &mut [u8],
    ) -> Result<[u8; CIPHER_TAG_LEN], CipherError> {
        let nonce = self.inner.build_nonce(nonce_ctr);
        Ok(self.inner.seal_in_place(&nonce, aad, in_out)?)
    }

    /// Encrypt in-place and append the 16-byte tag to the buffer.
    /// After this call, `buf` contains `[ciphertext || tag]` — ready for
    /// `open_in_place` with no intermediate allocation or copy.
    ///
    /// This is the zero-copy symmetric counterpart to `open_in_place`.
    /// Use this instead of `seal_in_place` + manual `extend_from_slice(&tag)`
    /// to avoid the footgun of separating ciphertext from tag.
    pub fn seal_in_place_append_tag(
        &self,
        nonce_ctr: u64,
        aad: &[u8],
        buf: &mut Vec<u8>,
    ) -> Result<(), CipherError> {
        let tag = self.seal_in_place(nonce_ctr, aad, buf)?;
        buf.extend_from_slice(&tag);
        Ok(())
    }

    /// Decrypt in-place from `[ciphertext || 16-byte tag]` buffer.
    /// Returns plaintext length.
    pub fn open_in_place(
        &self,
        nonce_ctr: u64,
        aad: &[u8],
        ct_and_tag: &mut [u8],
    ) -> Result<usize, CipherError> {
        if ct_and_tag.len() < CIPHER_TAG_LEN {
            return Err(CipherError::BufferTooSmall {
                need: CIPHER_TAG_LEN,
                have: ct_and_tag.len(),
            });
        }
        let nonce = self.inner.build_nonce(nonce_ctr);
        Ok(self.inner.open_in_place(&nonce, aad, ct_and_tag)?)
    }

    /// Decrypt with detached tag — zero allocation, zero memmove.
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
        if tag.len() < CIPHER_TAG_LEN {
            return Err(CipherError::BufferTooSmall {
                need: CIPHER_TAG_LEN,
                have: tag.len(),
            });
        }
        let nonce = self.inner.build_nonce(nonce_ctr);
        self.inner
            .open_detached(&nonce, aad, ciphertext, tag, plaintext_out)
            .map_err(CipherError::from)
    }
}

static_assertions::assert_impl_all!(BulkCipher: Send, Sync);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let cipher = BulkCipher::new(&[0x42; 32]);
        let original = b"hello world".to_vec();
        let mut buf = original.clone();
        let tag = cipher.seal_in_place(0, b"aad", &mut buf).unwrap();
        let mut combined = Vec::with_capacity(buf.len() + CIPHER_TAG_LEN);
        combined.extend_from_slice(&buf);
        combined.extend_from_slice(&tag);
        let pt_len = cipher.open_in_place(0, b"aad", &mut combined).unwrap();
        assert_eq!(&combined[..pt_len], &original);
    }

    #[test]
    fn tampered_rejected() {
        let cipher = BulkCipher::new(&[0x42; 32]);
        let mut buf = vec![0xAB; 1024];
        let tag = cipher.seal_in_place(0, b"", &mut buf).unwrap();
        buf[0] ^= 0xFF;
        let mut combined = buf.clone();
        combined.extend_from_slice(&tag);
        assert!(cipher.open_in_place(0, b"", &mut combined).is_err());
    }
}
