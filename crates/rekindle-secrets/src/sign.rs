//! Ed25519 signature verification for gossip envelopes.
//!
//! Used by the reader-validates model: every gossip message is signed by the
//! sender's pseudonym key, and every receiver verifies before processing.

use ed25519_dalek::{Signature, VerifyingKey};
use rekindle_types::error::CryptoError;

/// Verify an Ed25519 signature against a public key.
///
/// - `public_key_bytes`: The signer's Ed25519 public key (32 bytes).
/// - `data`: The signed data (typically serialized envelope bytes).
/// - `signature_bytes`: The 64-byte Ed25519 signature.
pub fn verify_signature(
    public_key_bytes: &[u8; 32],
    data: &[u8],
    signature_bytes: &[u8; 64],
) -> Result<(), CryptoError> {
    let verifying_key = VerifyingKey::from_bytes(public_key_bytes)
        .map_err(|e| CryptoError::InvalidKey(format!("invalid Ed25519 public key: {e}")))?;
    let signature = Signature::from_bytes(signature_bytes);
    use ed25519_dalek::Verifier;
    verifying_key
        .verify(data, &signature)
        .map_err(|e| CryptoError::Verification(format!("signature verification failed: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::derive::{derive_community_pseudonym, sign_with_pseudonym};

    #[test]
    fn sign_then_verify() {
        let key = derive_community_pseudonym(&[1u8; 32], "c");
        let data = b"hello world";
        let sig = sign_with_pseudonym(&key, data);
        let pub_bytes = key.verifying_key().to_bytes();
        assert!(verify_signature(&pub_bytes, data, &sig).is_ok());
    }

    #[test]
    fn wrong_data_fails() {
        let key = derive_community_pseudonym(&[1u8; 32], "c");
        let sig = sign_with_pseudonym(&key, b"hello");
        let pub_bytes = key.verifying_key().to_bytes();
        assert!(verify_signature(&pub_bytes, b"wrong", &sig).is_err());
    }

    #[test]
    fn wrong_key_fails() {
        let key = derive_community_pseudonym(&[1u8; 32], "c");
        let wrong_key = derive_community_pseudonym(&[2u8; 32], "c");
        let sig = sign_with_pseudonym(&key, b"hello");
        let wrong_pub = wrong_key.verifying_key().to_bytes();
        assert!(verify_signature(&wrong_pub, b"hello", &sig).is_err());
    }
}
