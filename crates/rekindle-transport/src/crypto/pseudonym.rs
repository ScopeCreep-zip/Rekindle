//! Per-community pseudonym derivation.
//!
//! Deterministically derives a unique, unlinkable Ed25519 keypair for each
//! community from the user's master secret. No correlation between a user's
//! pseudonyms across communities.

use ed25519_dalek::SigningKey;
use hkdf::Hkdf;
use sha2::Sha256;
use x25519_dalek::StaticSecret;

/// Derive a unique Ed25519 keypair for a specific community.
///
/// Uses HKDF-SHA256 with a fixed salt. Same inputs always produce
/// the same pseudonym — no storage needed, just re-derive.
pub fn derive_community_pseudonym(master_secret: &[u8; 32], community_id: &str) -> SigningKey {
    let hkdf = Hkdf::<Sha256>::new(Some(b"rekindle-community-pseudonym-v1"), master_secret);
    let mut seed = [0u8; 32];
    hkdf.expand(community_id.as_bytes(), &mut seed)
        .expect("32-byte output is a valid HKDF-SHA256 length");
    SigningKey::from_bytes(&seed)
}

/// Convert a pseudonym Ed25519 signing key to an X25519 static secret.
///
/// Uses the SHA-512-expanded scalar so the derived X25519 public key
/// matches the Edwards→Montgomery conversion of the Ed25519 public key.
pub fn pseudonym_to_x25519(key: &SigningKey) -> StaticSecret {
    StaticSecret::from(key.to_scalar_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::Verifier;

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
        let k1 = derive_community_pseudonym(&[1u8; 32], "c");
        let k2 = derive_community_pseudonym(&[2u8; 32], "c");
        assert_ne!(k1.to_bytes(), k2.to_bytes());
    }

    #[test]
    fn produces_valid_signing_key() {
        let key = derive_community_pseudonym(&[99u8; 32], "test");
        let verifying = key.verifying_key();
        use ed25519_dalek::Signer;
        let sig = key.sign(b"test message");
        assert!(verifying.verify(b"test message", &sig).is_ok());
    }

    #[test]
    fn x25519_conversion_does_not_panic() {
        let key = derive_community_pseudonym(&[77u8; 32], "test");
        let _x = pseudonym_to_x25519(&key);
    }
}
