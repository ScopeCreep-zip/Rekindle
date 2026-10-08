//! Per-community pseudonym derivation — façade over `rekindle_secrets::derive`.
//!
//! The single implementation lives in `rekindle-secrets` (the Tier-2
//! security boundary); this module re-exports it for the many
//! `rekindle_crypto::group::pseudonym::*` call sites. It used to carry a
//! wire-compatible copy of the same HKDF derivation, kept in sync by hand
//! with the secrets and transport copies — the tests below stay as a
//! regression guard that the derivation contract holds.

pub use rekindle_secrets::derive::{
    derive_community_pseudonym, pseudonym_to_x25519, sign_with_pseudonym,
};

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::VerifyingKey;

    #[test]
    fn deterministic_derivation() {
        let secret = [42u8; 32];
        let k1 = derive_community_pseudonym(&secret, "community_abc");
        let k2 = derive_community_pseudonym(&secret, "community_abc");
        assert_eq!(k1.to_bytes(), k2.to_bytes());
    }

    #[test]
    fn different_communities_different_keys() {
        let secret = [42u8; 32];
        let k1 = derive_community_pseudonym(&secret, "community_abc");
        let k2 = derive_community_pseudonym(&secret, "community_xyz");
        assert_ne!(k1.to_bytes(), k2.to_bytes());
    }

    #[test]
    fn different_secrets_different_keys() {
        let s1 = [1u8; 32];
        let s2 = [2u8; 32];
        let k1 = derive_community_pseudonym(&s1, "community_abc");
        let k2 = derive_community_pseudonym(&s2, "community_abc");
        assert_ne!(k1.to_bytes(), k2.to_bytes());
    }

    #[test]
    fn produces_valid_signing_key() {
        let secret = [99u8; 32];
        let key = derive_community_pseudonym(&secret, "test_community");
        let verifying = VerifyingKey::from(&key);
        // Verify we can sign and verify with the derived key
        use ed25519_dalek::Signer;
        let sig = key.sign(b"test message");
        assert!(verifying.verify_strict(b"test message", &sig).is_ok());
    }

    #[test]
    fn x25519_conversion() {
        let secret = [77u8; 32];
        let key = derive_community_pseudonym(&secret, "test_community");
        let _x25519_secret = pseudonym_to_x25519(&key);
        // Just verifying conversion doesn't panic
    }

    #[test]
    fn sign_with_pseudonym_verifies() {
        let key = derive_community_pseudonym(&[5u8; 32], "c");
        let sig = sign_with_pseudonym(&key, b"payload");
        let vk = VerifyingKey::from(&key);
        let sig = ed25519_dalek::Signature::from_bytes(&sig);
        assert!(vk.verify_strict(b"payload", &sig).is_ok());
    }
}
