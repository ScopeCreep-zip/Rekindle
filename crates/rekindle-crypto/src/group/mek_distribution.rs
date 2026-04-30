//! MEK (Media Encryption Key) distribution via X25519 ECDH + HKDF + AES-256-GCM.
//!
//! When a coordinator needs to distribute a channel's MEK to a member, it:
//! 1. Derives a shared secret via X25519 ECDH between the coordinator's
//!    pseudonym signing key and the member's pseudonym public key.
//! 2. Derives an AES-256-GCM wrapping key from the shared secret using HKDF-SHA256.
//! 3. Encrypts the MEK wire bytes (40 bytes: 8-byte generation LE + 32-byte key)
//!    with AES-256-GCM. Output: `[12-byte nonce || ciphertext+tag]` (68 bytes total).

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use ed25519_dalek::{SigningKey, VerifyingKey};
use hkdf::Hkdf;
use rand::RngCore;
use sha2::Sha256;
use x25519_dalek::PublicKey as X25519PublicKey;

use crate::error::CryptoError;

/// HKDF info label for MEK wrapping key derivation.
const HKDF_INFO: &[u8] = b"rekindle-mek-wrap-v1";

/// Derive an AES-256-GCM wrapping key from an X25519 shared secret.
fn derive_wrapping_key(shared_secret: &x25519_dalek::SharedSecret) -> [u8; 32] {
    let hkdf = Hkdf::<Sha256>::new(None, shared_secret.as_bytes());
    let mut key = [0u8; 32];
    hkdf.expand(HKDF_INFO, &mut key)
        .expect("32-byte output is valid for HKDF-SHA256");
    key
}

/// Wrap (encrypt) MEK wire bytes for a specific recipient.
///
/// - `sender_signing_key`: The coordinator's pseudonym Ed25519 signing key.
/// - `recipient_ed25519_public`: The target member's pseudonym Ed25519 public key bytes.
/// - `mek_wire_bytes`: The 40-byte MEK wire format (generation LE + key material).
///
/// Returns: `[12-byte nonce || ciphertext+tag]` (68 bytes for a 40-byte input).
pub fn wrap_mek(
    sender_signing_key: &SigningKey,
    recipient_ed25519_public: &[u8; 32],
    mek_wire_bytes: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    // Convert sender's Ed25519 signing key to X25519 static secret
    let sender_x25519 = super::pseudonym::pseudonym_to_x25519(sender_signing_key);

    // Convert recipient's Ed25519 public key to X25519 public key
    let recipient_verifying = VerifyingKey::from_bytes(recipient_ed25519_public).map_err(|e| {
        CryptoError::InvalidKey(format!("invalid recipient Ed25519 public key: {e}"))
    })?;
    let recipient_x25519 = X25519PublicKey::from(recipient_verifying.to_montgomery().to_bytes());

    // X25519 ECDH
    let shared_secret = sender_x25519.diffie_hellman(&recipient_x25519);

    // Derive wrapping key via HKDF
    let wrapping_key = derive_wrapping_key(&shared_secret);

    // AES-256-GCM encrypt
    let cipher = Aes256Gcm::new_from_slice(&wrapping_key)
        .map_err(|e| CryptoError::EncryptionError(format!("AES-GCM init: {e}")))?;

    let mut nonce_bytes = [0u8; 12];
    rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, mek_wire_bytes)
        .map_err(|e| CryptoError::EncryptionError(format!("AES-GCM encrypt: {e}")))?;

    // Output: [12-byte nonce || ciphertext+tag]
    let mut output = Vec::with_capacity(12 + ciphertext.len());
    output.extend_from_slice(&nonce_bytes);
    output.extend_from_slice(&ciphertext);
    Ok(output)
}

/// Unwrap (decrypt) MEK wire bytes received from a coordinator.
///
/// - `recipient_signing_key`: Our pseudonym Ed25519 signing key.
/// - `sender_ed25519_public`: The coordinator's pseudonym Ed25519 public key bytes.
/// - `wrapped_mek`: The encrypted MEK (`[12-byte nonce || ciphertext+tag]`).
///
/// Returns: The decrypted MEK wire bytes (40 bytes).
pub fn unwrap_mek(
    recipient_signing_key: &SigningKey,
    sender_ed25519_public: &[u8; 32],
    wrapped_mek: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    if wrapped_mek.len() < 12 {
        return Err(CryptoError::DecryptionError(
            "wrapped MEK too short".to_string(),
        ));
    }

    // Convert our signing key to X25519 static secret
    let recipient_x25519 = super::pseudonym::pseudonym_to_x25519(recipient_signing_key);

    // Convert sender's Ed25519 public key to X25519 public key
    let sender_verifying = VerifyingKey::from_bytes(sender_ed25519_public)
        .map_err(|e| CryptoError::InvalidKey(format!("invalid sender Ed25519 public key: {e}")))?;
    let sender_x25519 = X25519PublicKey::from(sender_verifying.to_montgomery().to_bytes());

    // X25519 ECDH (same shared secret due to commutativity)
    let shared_secret = recipient_x25519.diffie_hellman(&sender_x25519);

    // Derive wrapping key via HKDF
    let wrapping_key = derive_wrapping_key(&shared_secret);

    // AES-256-GCM decrypt
    let cipher = Aes256Gcm::new_from_slice(&wrapping_key)
        .map_err(|e| CryptoError::DecryptionError(format!("AES-GCM init: {e}")))?;

    let nonce = Nonce::from_slice(&wrapped_mek[..12]);
    let ciphertext = &wrapped_mek[12..];

    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| CryptoError::DecryptionError(format!("AES-GCM decrypt: {e}")))?
        .pipe(Ok)
}

/// Extension trait for piping values (avoids a temporary variable).
trait Pipe: Sized {
    fn pipe<R>(self, f: impl FnOnce(Self) -> R) -> R {
        f(self)
    }
}
impl<T> Pipe for T {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::group::media_key::MediaEncryptionKey;
    use crate::group::pseudonym::derive_community_pseudonym;

    #[test]
    fn wrap_unwrap_roundtrip() {
        let coordinator_secret = [1u8; 32];
        let member_secret = [2u8; 32];
        let community_id = "test_community";

        let coordinator_key = derive_community_pseudonym(&coordinator_secret, community_id);
        let member_key = derive_community_pseudonym(&member_secret, community_id);

        let mek = MediaEncryptionKey::generate(42);
        let mek_wire = mek.to_wire_bytes();

        // Coordinator wraps MEK for member
        let wrapped = wrap_mek(
            &coordinator_key,
            &member_key.verifying_key().to_bytes(),
            &mek_wire,
        )
        .unwrap();

        // Expected: 12 nonce + 40 plaintext + 16 tag = 68 bytes
        assert_eq!(wrapped.len(), 68);

        // Member unwraps MEK
        let unwrapped = unwrap_mek(
            &member_key,
            &coordinator_key.verifying_key().to_bytes(),
            &wrapped,
        )
        .unwrap();

        assert_eq!(unwrapped, mek_wire);

        // Verify the unwrapped MEK has correct generation and key material
        let restored = MediaEncryptionKey::from_wire_bytes(&unwrapped).unwrap();
        assert_eq!(restored.generation(), 42);
        assert_eq!(restored.as_bytes(), mek.as_bytes());
    }

    #[test]
    fn wrong_recipient_cannot_unwrap() {
        let coordinator_secret = [1u8; 32];
        let member_secret = [2u8; 32];
        let wrong_member_secret = [3u8; 32];
        let community_id = "test_community";

        let coordinator_key = derive_community_pseudonym(&coordinator_secret, community_id);
        let member_key = derive_community_pseudonym(&member_secret, community_id);
        let wrong_member_key = derive_community_pseudonym(&wrong_member_secret, community_id);

        let mek = MediaEncryptionKey::generate(1);
        let wrapped = wrap_mek(
            &coordinator_key,
            &member_key.verifying_key().to_bytes(),
            &mek.to_wire_bytes(),
        )
        .unwrap();

        // Wrong member tries to unwrap — should fail
        let result = unwrap_mek(
            &wrong_member_key,
            &coordinator_key.verifying_key().to_bytes(),
            &wrapped,
        );
        assert!(result.is_err());
    }

    #[test]
    fn wrong_sender_key_cannot_unwrap() {
        let coordinator_secret = [1u8; 32];
        let fake_coordinator_secret = [99u8; 32];
        let member_secret = [2u8; 32];
        let community_id = "test_community";

        let coordinator_key = derive_community_pseudonym(&coordinator_secret, community_id);
        let fake_coordinator_key =
            derive_community_pseudonym(&fake_coordinator_secret, community_id);
        let member_key = derive_community_pseudonym(&member_secret, community_id);

        let mek = MediaEncryptionKey::generate(1);
        let wrapped = wrap_mek(
            &coordinator_key,
            &member_key.verifying_key().to_bytes(),
            &mek.to_wire_bytes(),
        )
        .unwrap();

        // Member tries to unwrap with wrong sender public key — should fail
        let result = unwrap_mek(
            &member_key,
            &fake_coordinator_key.verifying_key().to_bytes(),
            &wrapped,
        );
        assert!(result.is_err());
    }

    #[test]
    fn wrapped_mek_too_short() {
        let coordinator_secret = [1u8; 32];
        let member_secret = [2u8; 32];
        let coordinator_key = derive_community_pseudonym(&coordinator_secret, "c");
        let member_key = derive_community_pseudonym(&member_secret, "c");

        let result = unwrap_mek(
            &member_key,
            &coordinator_key.verifying_key().to_bytes(),
            &[0u8; 11], // too short
        );
        assert!(result.is_err());
    }

    #[test]
    fn different_communities_different_wrapping_keys() {
        let coordinator_secret = [1u8; 32];
        let member_secret = [2u8; 32];

        let coord_a = derive_community_pseudonym(&coordinator_secret, "community_a");
        let member_a = derive_community_pseudonym(&member_secret, "community_a");

        let coord_b = derive_community_pseudonym(&coordinator_secret, "community_b");
        let member_b = derive_community_pseudonym(&member_secret, "community_b");

        let mek = MediaEncryptionKey::generate(1);
        let wrapped_a = wrap_mek(
            &coord_a,
            &member_a.verifying_key().to_bytes(),
            &mek.to_wire_bytes(),
        )
        .unwrap();

        // Cannot unwrap community_a's MEK with community_b's keys
        let result = unwrap_mek(&member_b, &coord_b.verifying_key().to_bytes(), &wrapped_a);
        assert!(result.is_err());
    }

    #[test]
    fn output_size_is_68_bytes() {
        let sender = derive_community_pseudonym(&[1u8; 32], "c");
        let recipient = derive_community_pseudonym(&[2u8; 32], "c");
        let mek = MediaEncryptionKey::generate(1);
        let wire = mek.to_wire_bytes();
        assert_eq!(wire.len(), 40);

        let wrapped = wrap_mek(&sender, &recipient.verifying_key().to_bytes(), &wire).unwrap();
        // 12 nonce + 40 plaintext + 16 GCM tag = 68
        assert_eq!(wrapped.len(), 68);
    }
}
