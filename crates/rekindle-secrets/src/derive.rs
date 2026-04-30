//! Key derivation functions — pseudonyms and SMPL slot keypairs.
//!
//! Pseudonyms: HKDF-SHA256(master_secret, community_id) → Ed25519 keypair.
//! Slot keypairs: HKDF-SHA256(slot_seed, "rekindle-slot-{index}") → Ed25519 keypair.

use ed25519_dalek::SigningKey;
use hkdf::Hkdf;
use rekindle_types::error::CryptoError;
use sha2::Sha256;
use x25519_dalek::StaticSecret;

/// Derive a unique, unlinkable Ed25519 keypair for a specific community.
///
/// Uses HKDF-SHA256 with a fixed salt. Deterministic: same inputs always
/// produce the same keypair. Different community IDs produce different
/// keypairs, making cross-community identity correlation impossible.
pub fn derive_community_pseudonym(master_secret: &[u8; 32], community_id: &str) -> SigningKey {
    let hkdf = Hkdf::<Sha256>::new(Some(b"rekindle-community-pseudonym-v1"), master_secret);
    let mut seed = [0u8; 32];
    hkdf.expand(community_id.as_bytes(), &mut seed)
        .expect("32-byte output is a valid HKDF-SHA256 length");
    SigningKey::from_bytes(&seed)
}

/// Sign arbitrary bytes with a pseudonym key, returning a 64-byte Ed25519 signature.
pub fn sign_with_pseudonym(signing_key: &SigningKey, data: &[u8]) -> [u8; 64] {
    use ed25519_dalek::Signer;
    signing_key.sign(data).to_bytes()
}

/// Convert a pseudonym Ed25519 signing key to an X25519 static secret.
///
/// Used for ECDH key agreement (MEK wrapping, call key derivation).
/// The conversion uses `to_scalar_bytes()` which extracts the scalar
/// from the SHA-512 expansion of the Ed25519 secret key.
pub fn pseudonym_to_x25519(key: &SigningKey) -> StaticSecret {
    StaticSecret::from(key.to_scalar_bytes())
}

/// Derive a deterministic Ed25519 keypair for a SMPL member slot.
///
/// Uses HKDF-SHA256(seed, "rekindle-slot-{slot}") → 32 bytes → Ed25519 keypair.
/// Every community member knows the slot_seed (from InviteSecrets), so any
/// member can derive any slot's keypair — this is by design for the
/// universal SMPL schema (Q-pid equation).
pub fn derive_slot_keypair(seed: &[u8; 32], slot: u32) -> Result<SigningKey, CryptoError> {
    let hk = Hkdf::<Sha256>::new(None, seed);
    let info = format!("rekindle-slot-{slot}");
    let mut okm = [0u8; 32];
    hk.expand(info.as_bytes(), &mut okm)
        .map_err(|_| CryptoError::KeyGeneration("HKDF expand failed for slot keypair".into()))?;
    Ok(SigningKey::from_bytes(&okm))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, Verifier, VerifyingKey};

    #[test]
    fn pseudonym_deterministic() {
        let secret = [42u8; 32];
        let k1 = derive_community_pseudonym(&secret, "community_abc");
        let k2 = derive_community_pseudonym(&secret, "community_abc");
        assert_eq!(k1.to_bytes(), k2.to_bytes());
    }

    #[test]
    fn pseudonym_unlinkable_across_communities() {
        let secret = [42u8; 32];
        let k1 = derive_community_pseudonym(&secret, "community_abc");
        let k2 = derive_community_pseudonym(&secret, "community_xyz");
        assert_ne!(k1.to_bytes(), k2.to_bytes());
    }

    #[test]
    fn pseudonym_different_secrets() {
        let k1 = derive_community_pseudonym(&[1u8; 32], "c");
        let k2 = derive_community_pseudonym(&[2u8; 32], "c");
        assert_ne!(k1.to_bytes(), k2.to_bytes());
    }

    #[test]
    fn pseudonym_produces_valid_key() {
        let key = derive_community_pseudonym(&[99u8; 32], "test");
        let verifying = VerifyingKey::from(&key);
        let sig = key.sign(b"test message");
        assert!(verifying.verify(b"test message", &sig).is_ok());
    }

    #[test]
    fn pseudonym_x25519_conversion() {
        let key = derive_community_pseudonym(&[77u8; 32], "test");
        let _x25519 = pseudonym_to_x25519(&key);
    }

    #[test]
    fn sign_and_verify() {
        let key = derive_community_pseudonym(&[1u8; 32], "c");
        let sig = sign_with_pseudonym(&key, b"hello");
        let verifying = VerifyingKey::from(&key);
        let signature = ed25519_dalek::Signature::from_bytes(&sig);
        assert!(verifying.verify(b"hello", &signature).is_ok());
    }

    #[test]
    fn slot_keypair_deterministic() {
        let seed = [42u8; 32];
        let k1 = derive_slot_keypair(&seed, 0).unwrap();
        let k2 = derive_slot_keypair(&seed, 0).unwrap();
        assert_eq!(k1.to_bytes(), k2.to_bytes());
    }

    #[test]
    fn slot_keypair_different_slots() {
        let seed = [42u8; 32];
        let k1 = derive_slot_keypair(&seed, 0).unwrap();
        let k2 = derive_slot_keypair(&seed, 1).unwrap();
        assert_ne!(k1.to_bytes(), k2.to_bytes());
    }

    #[test]
    fn slot_keypair_different_seeds() {
        let k1 = derive_slot_keypair(&[1u8; 32], 0).unwrap();
        let k2 = derive_slot_keypair(&[2u8; 32], 0).unwrap();
        assert_ne!(k1.to_bytes(), k2.to_bytes());
    }

    #[test]
    fn slot_keypair_valid_range() {
        let seed = [1u8; 32];
        // Should work for all 255 SMPL slots
        for i in 0..255u32 {
            assert!(derive_slot_keypair(&seed, i).is_ok());
        }
    }
}
