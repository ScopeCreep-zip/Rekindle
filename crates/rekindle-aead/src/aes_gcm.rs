//! AES-256-GCM implementation of `BulkAead` via aws-lc-rs.
//!
//! Uses `LessSafeKey` which holds the pre-expanded key schedule and
//! H-powers table for the session lifetime. The `open_separate_gather`
//! method calls `EVP_AEAD_CTX_open_gather` directly — zero allocation,
//! zero memmove on the decrypt path.

use aws_lc_rs::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, NONCE_LEN};
use crate::traits::{AeadAlgorithm, AeadError, BulkAead};

/// AES-256-GCM AEAD key.
///
/// Wraps `LessSafeKey` which is `Send + Sync`. The key schedule and
/// H-powers table are computed once at construction and reused for
/// all seal/open calls.
pub struct AesGcmKey {
    key: LessSafeKey,
}

static_assertions::assert_impl_all!(AesGcmKey: Send, Sync);

impl AesGcmKey {
    /// Construct from a 32-byte AES-256 key.
    pub fn new(key_bytes: &[u8; 32]) -> Result<Self, AeadError> {
        let unbound = UnboundKey::new(&AES_256_GCM, key_bytes)
            .map_err(|_| AeadError::Init)?;
        Ok(Self { key: LessSafeKey::new(unbound) })
    }
}

impl BulkAead for AesGcmKey {
    fn algorithm(&self) -> AeadAlgorithm { AeadAlgorithm::Aes256Gcm }
    fn nonce_len(&self) -> usize { 12 }

    fn seal_detached(
        &self, nonce: &[u8], aad: &[u8],
        plaintext: &[u8], ciphertext_out: &mut [u8], tag_out: &mut [u8],
    ) -> Result<(), AeadError> {
        if nonce.len() < NONCE_LEN {
            return Err(AeadError::BufferSize { need: NONCE_LEN, have: nonce.len() });
        }
        let nonce = &nonce[..NONCE_LEN];
        if plaintext.len() != ciphertext_out.len() {
            return Err(AeadError::BufferSize { need: plaintext.len(), have: ciphertext_out.len() });
        }
        if tag_out.len() < 16 {
            return Err(AeadError::BufferSize { need: 16, have: tag_out.len() });
        }

        ciphertext_out.copy_from_slice(plaintext);
        let nonce_arr: [u8; NONCE_LEN] = nonce.try_into()
            .map_err(|_| AeadError::Init)?;
        let n = Nonce::assume_unique_for_key(nonce_arr);
        let tag = self.key
            .seal_in_place_separate_tag(n, Aad::from(aad), ciphertext_out)
            .map_err(|_| AeadError::AuthFailed)?;
        tag_out[..16].copy_from_slice(tag.as_ref());
        Ok(())
    }

    fn open_detached(
        &self, nonce: &[u8], aad: &[u8],
        ciphertext: &[u8], tag: &[u8], plaintext_out: &mut [u8],
    ) -> Result<(), AeadError> {
        if nonce.len() < NONCE_LEN {
            return Err(AeadError::BufferSize { need: NONCE_LEN, have: nonce.len() });
        }
        let nonce = &nonce[..NONCE_LEN];
        if ciphertext.len() != plaintext_out.len() {
            return Err(AeadError::BufferSize { need: ciphertext.len(), have: plaintext_out.len() });
        }
        if tag.len() < 16 {
            return Err(AeadError::BufferSize { need: 16, have: tag.len() });
        }

        let nonce_arr: [u8; NONCE_LEN] = nonce.try_into()
            .map_err(|_| AeadError::Init)?;
        let n = Nonce::assume_unique_for_key(nonce_arr);
        self.key
            .open_separate_gather(n, Aad::from(aad), ciphertext, tag, plaintext_out)
            .map_err(|_| AeadError::AuthFailed)
    }

    fn seal_in_place(
        &self, nonce: &[u8], aad: &[u8], in_out: &mut [u8],
    ) -> Result<[u8; 16], AeadError> {
        if nonce.len() < NONCE_LEN {
            return Err(AeadError::BufferSize { need: NONCE_LEN, have: nonce.len() });
        }
        let nonce = &nonce[..NONCE_LEN];
        let nonce_arr: [u8; NONCE_LEN] = nonce.try_into()
            .map_err(|_| AeadError::Init)?;
        let n = Nonce::assume_unique_for_key(nonce_arr);
        let tag = self.key
            .seal_in_place_separate_tag(n, Aad::from(aad), in_out)
            .map_err(|_| AeadError::AuthFailed)?;
        let mut tag_bytes = [0u8; 16];
        tag_bytes.copy_from_slice(tag.as_ref());
        Ok(tag_bytes)
    }

    fn open_in_place(
        &self, nonce: &[u8], aad: &[u8], ct_and_tag: &mut [u8],
    ) -> Result<usize, AeadError> {
        if nonce.len() < NONCE_LEN {
            return Err(AeadError::BufferSize { need: NONCE_LEN, have: nonce.len() });
        }
        let nonce = &nonce[..NONCE_LEN];
        if ct_and_tag.len() < 16 {
            return Err(AeadError::BufferSize { need: 16, have: ct_and_tag.len() });
        }
        let nonce_arr: [u8; NONCE_LEN] = nonce.try_into()
            .map_err(|_| AeadError::Init)?;
        let n = Nonce::assume_unique_for_key(nonce_arr);
        let plaintext = self.key
            .open_in_place(n, Aad::from(aad), ct_and_tag)
            .map_err(|_| AeadError::AuthFailed)?;
        Ok(plaintext.len())
    }

    fn build_nonce(&self, counter: u64) -> [u8; 16] {
        let mut nonce = [0u8; 16];
        nonce[4..12].copy_from_slice(&counter.to_be_bytes());
        nonce
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let key = AesGcmKey::new(&[0x42; 32]).unwrap();
        let nonce = key.build_nonce(0);
        let plain = vec![0xAB; 65519];
        let mut ct = vec![0u8; plain.len()];
        let mut tag = [0u8; 16];
        key.seal_detached(&nonce, b"aad", &plain, &mut ct, &mut tag).unwrap();
        let mut pt = vec![0u8; plain.len()];
        key.open_detached(&nonce, b"aad", &ct, &tag, &mut pt).unwrap();
        assert_eq!(pt, plain);
    }

    #[test]
    fn tamper_rejected() {
        let key = AesGcmKey::new(&[0x42; 32]).unwrap();
        let nonce = key.build_nonce(1);
        let plain = vec![0xCD; 1024];
        let mut ct = vec![0u8; 1024];
        let mut tag = [0u8; 16];
        key.seal_detached(&nonce, b"", &plain, &mut ct, &mut tag).unwrap();
        ct[0] ^= 0xFF;
        let mut pt = vec![0u8; 1024];
        assert!(key.open_detached(&nonce, b"", &ct, &tag, &mut pt).is_err());
    }

    #[test]
    fn wrong_nonce_rejected() {
        let key = AesGcmKey::new(&[0x42; 32]).unwrap();
        let nonce0 = key.build_nonce(0);
        let nonce1 = key.build_nonce(1);
        let plain = vec![0xEF; 512];
        let mut ct = vec![0u8; 512];
        let mut tag = [0u8; 16];
        key.seal_detached(&nonce0, b"", &plain, &mut ct, &mut tag).unwrap();
        let mut pt = vec![0u8; 512];
        assert!(key.open_detached(&nonce1, b"", &ct, &tag, &mut pt).is_err());
    }

    #[test]
    fn wrong_aad_rejected() {
        let key = AesGcmKey::new(&[0x42; 32]).unwrap();
        let nonce = key.build_nonce(0);
        let plain = vec![0xAB; 512];
        let mut ct = vec![0u8; 512];
        let mut tag = [0u8; 16];
        key.seal_detached(&nonce, b"correct", &plain, &mut ct, &mut tag).unwrap();
        let mut pt = vec![0u8; 512];
        assert!(key.open_detached(&nonce, b"wrong", &ct, &tag, &mut pt).is_err());
    }

    #[test]
    fn wrong_key_rejected() {
        let key1 = AesGcmKey::new(&[0x42; 32]).unwrap();
        let key2 = AesGcmKey::new(&[0x43; 32]).unwrap();
        let nonce = key1.build_nonce(0);
        let plain = vec![0xAB; 512];
        let mut ct = vec![0u8; 512];
        let mut tag = [0u8; 16];
        key1.seal_detached(&nonce, b"", &plain, &mut ct, &mut tag).unwrap();
        let mut pt = vec![0u8; 512];
        assert!(key2.open_detached(&nonce, b"", &ct, &tag, &mut pt).is_err());
    }

    #[test]
    fn zero_length_plaintext_roundtrip() {
        let key = AesGcmKey::new(&[0x42; 32]).unwrap();
        let nonce = key.build_nonce(0);
        let plain = b"";
        let mut ct = vec![0u8; 0];
        let mut tag = [0u8; 16];
        key.seal_detached(&nonce, b"aad", plain, &mut ct, &mut tag).unwrap();
        let mut pt = vec![0u8; 0];
        key.open_detached(&nonce, b"aad", &ct, &tag, &mut pt).unwrap();
        assert!(pt.is_empty());
    }

    #[test]
    fn max_u64_nonce_roundtrip() {
        let key = AesGcmKey::new(&[0x42; 32]).unwrap();
        let nonce = key.build_nonce(u64::MAX);
        assert_eq!(nonce.len(), 16);
        let plain = b"max nonce test";
        let mut ct = vec![0u8; plain.len()];
        let mut tag = [0u8; 16];
        key.seal_detached(&nonce, b"", plain, &mut ct, &mut tag).unwrap();
        let mut pt = vec![0u8; plain.len()];
        key.open_detached(&nonce, b"", &ct, &tag, &mut pt).unwrap();
        assert_eq!(&pt, plain);
    }

    #[test]
    fn build_nonce_returns_16_bytes() {
        let key = AesGcmKey::new(&[0x42; 32]).unwrap();
        assert_eq!(key.build_nonce(0).len(), 16);
        assert_eq!(key.build_nonce(u64::MAX).len(), 16);
        // GCM uses first 12 bytes; implementation slices internally.
        assert_eq!(key.nonce_len(), 12);
    }

    #[test]
    fn open_detached_zeroes_output_on_failure() {
        let key = AesGcmKey::new(&[0x42; 32]).unwrap();
        let nonce = key.build_nonce(0);
        let plain = vec![0xAB; 256];
        let mut ct = vec![0u8; 256];
        let mut tag = [0u8; 16];
        key.seal_detached(&nonce, b"", &plain, &mut ct, &mut tag).unwrap();
        ct[0] ^= 0xFF; // tamper
        let mut pt = vec![0xCC; 256]; // pre-fill with non-zero
        assert!(key.open_detached(&nonce, b"", &ct, &tag, &mut pt).is_err());
        // aws-lc zeroes the output buffer on auth failure
        assert!(pt.iter().all(|&b| b == 0), "output must be zeroed on auth failure");
    }

    #[test]
    fn concurrent_seal_uniqueness() {
        use std::sync::Arc;
        let key = Arc::new(AesGcmKey::new(&[0x42; 32]).unwrap());
        let n = 100usize;
        let handles: Vec<_> = (0..8).map(|t| {
            let key = Arc::clone(&key);
            std::thread::spawn(move || {
                let mut results = Vec::with_capacity(n);
                for i in 0..n {
                    let counter = (t * n + i) as u64;
                    let nonce = key.build_nonce(counter);
                    let plain = vec![0xABu8; 64];
                    let mut ct = vec![0u8; 64];
                    let mut tag = [0u8; 16];
                    key.seal_detached(&nonce, b"", &plain, &mut ct, &mut tag).unwrap();
                    results.push((ct, tag));
                }
                results
            })
        }).collect();

        let mut all: Vec<(Vec<u8>, [u8; 16])> = Vec::new();
        for h in handles {
            all.extend(h.join().unwrap());
        }
        // All ciphertexts must be distinct (different nonces → different output)
        for i in 0..all.len() {
            for j in (i+1)..all.len() {
                assert_ne!(all[i], all[j], "ciphertexts {i} and {j} collided");
            }
        }
    }
}
