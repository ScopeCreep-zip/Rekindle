//! Unified AEAD interface for the Rekindle bulk transport plane.
//!
//! Provides the `BulkAead` trait with implementations for:
//! - **AES-256-GCM** (default) — via aws-lc-rs `LessSafeKey`, ~3.6 GiB/s seal on Coffee Lake
//! - **AEGIS-128L** (opt-in, `aegis` feature) — via vendored libaegis FFI, ~6–8 GiB/s
//!
//! Both implementations are `Send + Sync` and designed for concurrent use
//! from rayon workers via `Arc<dyn BulkAead>`.
//!
//! # Security
//!
//! - AEGIS-128L key material is wrapped in `Zeroizing` and zeroed on drop
//! - AES-256-GCM key material is zeroed on drop by aws-lc's `EVP_AEAD_CTX_cleanup`
//! - AEGIS-128L is structurally symmetric (encrypt ≈ decrypt by construction)
//! - AES-GCM uses `open_separate_gather` for zero-alloc decrypt
//! - Nonce construction is algorithm-aware (12B for GCM, 16B for AEGIS)

#![deny(unsafe_code)]

mod traits;
pub mod aes_gcm;

#[cfg(feature = "aegis")]
#[allow(unsafe_code)]
pub mod aegis128l;

pub use traits::{AeadAlgorithm, AeadError, BulkAead};

#[cfg(test)]
mod cross_algorithm_tests {
    use super::aes_gcm::AesGcmKey;
    use super::traits::BulkAead;

    /// Verify that GCM ciphertext cannot be decrypted by a different GCM key.
    /// This is a sanity check that key isolation holds — not algorithm-specific,
    /// but exercises the trait interface uniformly.
    #[test]
    fn gcm_key_isolation() {
        let key1 = AesGcmKey::new(&[0x42; 32]).unwrap();
        let key2 = AesGcmKey::new(&[0x43; 32]).unwrap();
        let nonce = key1.build_nonce(0);
        let plain = b"cross-key test";
        let mut ct = vec![0u8; plain.len()];
        let mut tag = [0u8; 16];
        key1.seal_detached(&nonce, b"", plain, &mut ct, &mut tag).unwrap();

        // Same nonce, different key — must fail
        let nonce2 = key2.build_nonce(0);
        let mut pt = vec![0u8; plain.len()];
        assert!(key2.open_detached(&nonce2, b"", &ct, &tag, &mut pt).is_err());
    }

    /// Verify trait contract: algorithm() returns the correct discriminant.
    #[test]
    fn algorithm_discriminant() {
        let gcm = AesGcmKey::new(&[0x42; 32]).unwrap();
        assert_eq!(gcm.algorithm(), super::AeadAlgorithm::Aes256Gcm);
        assert_eq!(gcm.nonce_len(), 12);
        assert_eq!(gcm.tag_len(), 16);
    }

    /// Verify trait contract: tag_len() default is 16 for both.
    #[cfg(feature = "aegis")]
    #[test]
    fn aegis_algorithm_discriminant() {
        use super::aegis128l::Aegis128LKey;
        let aegis = Aegis128LKey::new(&[0x42; 16]);
        assert_eq!(aegis.algorithm(), super::AeadAlgorithm::Aegis128L);
        assert_eq!(aegis.nonce_len(), 16);
        assert_eq!(aegis.tag_len(), 16);
    }

    /// Cross-algorithm: GCM ciphertext fed to AEGIS must fail.
    /// This tests that algorithm confusion is not silently accepted.
    #[cfg(feature = "aegis")]
    #[test]
    fn gcm_ciphertext_rejected_by_aegis() {
        use super::aegis128l::Aegis128LKey;

        let gcm_key = AesGcmKey::new(&[0x42; 32]).unwrap();
        let aegis_key = Aegis128LKey::new(&[0x42; 16]);

        let gcm_nonce = gcm_key.build_nonce(0);
        let plain = b"cross-algorithm test data";
        let mut ct = vec![0u8; plain.len()];
        let mut tag = [0u8; 16];
        gcm_key.seal_detached(&gcm_nonce, b"", plain, &mut ct, &mut tag).unwrap();

        // Feed GCM ciphertext to AEGIS with a 16-byte nonce
        let aegis_nonce = aegis_key.build_nonce(0);
        let mut pt = vec![0u8; plain.len()];
        assert!(aegis_key.open_detached(&aegis_nonce, b"", &ct, &tag, &mut pt).is_err());
    }
}
