//! MEK (Media Encryption Key) distribution via X25519 ECDH + HKDF + AES-256-GCM.
//!
//! Thin façade over `rekindle_secrets::mek` — the single wrap/unwrap
//! implementation shared by every track (same HKDF info label
//! `rekindle-mek-wrap-v1`, same `[12-byte nonce || ciphertext+tag]` wire
//! format, and a zeroizing wrapping key per audit finding P7-W26). This
//! module used to carry its own copy of the algorithm, wire-compatible by
//! hand with the secrets and transport copies; only the
//! `rekindle_types::error::CryptoError → crate::error::CryptoError`
//! mapping lives here now.
//!
//! Wire format: `[12-byte nonce || ciphertext + 16-byte tag]`
//! (68 bytes for the 40-byte MEK wire input: 8-byte generation LE + 32-byte key).

use ed25519_dalek::SigningKey;
use rekindle_secrets::CryptoError as SecretsCryptoError;

use crate::error::CryptoError;

/// Map the secrets-crate error taxonomy onto this crate's.
fn map_err(e: SecretsCryptoError) -> CryptoError {
    match e {
        SecretsCryptoError::Encryption(s) => CryptoError::EncryptionError(s),
        SecretsCryptoError::Decryption(s) => CryptoError::DecryptionError(s),
        SecretsCryptoError::InvalidKey(s) => CryptoError::InvalidKey(s),
        SecretsCryptoError::KeyGeneration(s) => CryptoError::KeyGeneration(s),
        SecretsCryptoError::Signing(s) => CryptoError::SigningError(s),
        SecretsCryptoError::Verification(s) => CryptoError::VerificationError(s),
        SecretsCryptoError::Storage(s) => CryptoError::StorageError(s),
    }
}

/// Wrap (encrypt) MEK wire bytes for a specific recipient.
///
/// - `sender_signing_key`: The wrapping peer's pseudonym Ed25519 signing key.
/// - `recipient_ed25519_public`: The target member's pseudonym Ed25519 public key bytes.
/// - `mek_wire_bytes`: The 40-byte MEK wire format (generation LE + key material).
///
/// Returns: `[12-byte nonce || ciphertext+tag]` (68 bytes for a 40-byte input).
pub fn wrap_mek(
    sender_signing_key: &SigningKey,
    recipient_ed25519_public: &[u8; 32],
    mek_wire_bytes: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    rekindle_secrets::mek::wrap_mek(sender_signing_key, recipient_ed25519_public, mek_wire_bytes)
        .map_err(map_err)
}

/// Unwrap (decrypt) MEK wire bytes received from a peer.
///
/// - `recipient_signing_key`: Our pseudonym Ed25519 signing key.
/// - `sender_ed25519_public`: The wrapping peer's pseudonym Ed25519 public key bytes.
/// - `wrapped_mek`: The encrypted MEK (`[12-byte nonce || ciphertext+tag]`).
///
/// Returns: The decrypted MEK wire bytes (40 bytes).
pub fn unwrap_mek(
    recipient_signing_key: &SigningKey,
    sender_ed25519_public: &[u8; 32],
    wrapped_mek: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    rekindle_secrets::mek::unwrap_mek(recipient_signing_key, sender_ed25519_public, wrapped_mek)
        .map_err(map_err)
}

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
