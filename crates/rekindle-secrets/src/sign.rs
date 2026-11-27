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
    verifying_key
        .verify_strict(data, &signature)
        .map_err(|e| CryptoError::Verification(format!("signature verification failed: {e}")))
}

#[cfg(test)]
#[path = "sign_tests.rs"]
mod tests;
