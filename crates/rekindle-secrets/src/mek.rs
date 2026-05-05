//! MEK (Media Encryption Key) wrapping via X25519 ECDH + HKDF + AES-256-GCM.
//!
//! Used for peer-to-peer MEK distribution. The sender (deterministic rotator)
//! wraps the MEK for each recipient using their pseudonym public key.
//! No coordinator involved — any peer can wrap/unwrap.
//!
//! Wire format: `[12-byte nonce || ciphertext + 16-byte tag]` (68 bytes for 40-byte MEK).

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use ed25519_dalek::{SigningKey, VerifyingKey};
use hkdf::Hkdf;
use rand::RngCore;
use rekindle_types::error::CryptoError;
use sha2::Sha256;
use x25519_dalek::PublicKey as X25519PublicKey;
use zeroize::Zeroizing;

use crate::derive::pseudonym_to_x25519;

/// HKDF info label for MEK wrapping key derivation.
const HKDF_INFO: &[u8] = b"rekindle-mek-wrap-v1";

/// Derive an AES-256-GCM wrapping key from an X25519 shared secret.
/// The return type wraps the bytes in `Zeroizing` so the wrapping key
/// is scrubbed from memory when it goes out of scope, even on the
/// happy path. Audit finding (P7-W26).
fn derive_wrapping_key(shared_secret: &x25519_dalek::SharedSecret) -> Zeroizing<[u8; 32]> {
    let hkdf = Hkdf::<Sha256>::new(None, shared_secret.as_bytes());
    let mut key = Zeroizing::new([0u8; 32]);
    hkdf.expand(HKDF_INFO, key.as_mut())
        .expect("32-byte output is valid for HKDF-SHA256");
    key
}

/// Wrap (encrypt) MEK wire bytes for a specific recipient.
///
/// - `sender_signing_key`: The wrapping peer's Ed25519 pseudonym key.
/// - `recipient_ed25519_public`: The target member's Ed25519 public key bytes.
/// - `mek_wire_bytes`: The 40-byte MEK wire format `[generation LE || key]`.
///
/// Returns: `[12-byte nonce || ciphertext + 16-byte tag]` (68 bytes).
pub fn wrap_mek(
    sender_signing_key: &SigningKey,
    recipient_ed25519_public: &[u8; 32],
    mek_wire_bytes: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    let sender_x25519 = pseudonym_to_x25519(sender_signing_key);

    let recipient_verifying = VerifyingKey::from_bytes(recipient_ed25519_public)
        .map_err(|e| CryptoError::InvalidKey(format!("invalid recipient Ed25519 key: {e}")))?;
    let recipient_x25519 = X25519PublicKey::from(recipient_verifying.to_montgomery().to_bytes());

    let shared_secret = sender_x25519.diffie_hellman(&recipient_x25519);
    let wrapping_key = derive_wrapping_key(&shared_secret);

    let cipher = Aes256Gcm::new_from_slice(&wrapping_key[..])
        .map_err(|e| CryptoError::Encryption(format!("AES-GCM init: {e}")))?;

    let mut nonce_bytes = [0u8; 12];
    rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, mek_wire_bytes)
        .map_err(|e| CryptoError::Encryption(format!("AES-GCM encrypt: {e}")))?;

    let mut output = Vec::with_capacity(12 + ciphertext.len());
    output.extend_from_slice(&nonce_bytes);
    output.extend_from_slice(&ciphertext);
    Ok(output)
}

/// Unwrap (decrypt) MEK wire bytes received from a peer.
///
/// - `recipient_signing_key`: Our Ed25519 pseudonym signing key.
/// - `sender_ed25519_public`: The wrapping peer's Ed25519 public key bytes.
/// - `wrapped_mek`: The encrypted MEK `[12-byte nonce || ciphertext + tag]`.
///
/// Returns: The decrypted MEK wire bytes (40 bytes).
pub fn unwrap_mek(
    recipient_signing_key: &SigningKey,
    sender_ed25519_public: &[u8; 32],
    wrapped_mek: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    if wrapped_mek.len() < 12 {
        return Err(CryptoError::Decryption("wrapped MEK too short".into()));
    }

    let recipient_x25519 = pseudonym_to_x25519(recipient_signing_key);

    let sender_verifying = VerifyingKey::from_bytes(sender_ed25519_public)
        .map_err(|e| CryptoError::InvalidKey(format!("invalid sender Ed25519 key: {e}")))?;
    let sender_x25519 = X25519PublicKey::from(sender_verifying.to_montgomery().to_bytes());

    let shared_secret = recipient_x25519.diffie_hellman(&sender_x25519);
    let wrapping_key = derive_wrapping_key(&shared_secret);

    let cipher = Aes256Gcm::new_from_slice(&wrapping_key[..])
        .map_err(|e| CryptoError::Decryption(format!("AES-GCM init: {e}")))?;

    let nonce = Nonce::from_slice(&wrapped_mek[..12]);
    cipher
        .decrypt(nonce, &wrapped_mek[12..])
        .map_err(|e| CryptoError::Decryption(format!("AES-GCM decrypt: {e}")))?
        .pipe(Ok)
}

/// Extension trait for method chaining.
trait Pipe: Sized {
    fn pipe<R>(self, f: impl FnOnce(Self) -> R) -> R {
        f(self)
    }
}
impl<T> Pipe for T {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::derive::derive_community_pseudonym;
    use crate::keys::MediaEncryptionKey;

    #[test]
    fn wrap_unwrap_roundtrip() {
        let sender_key = derive_community_pseudonym(&[1u8; 32], "test");
        let recipient_key = derive_community_pseudonym(&[2u8; 32], "test");
        let mek = MediaEncryptionKey::generate(42);
        let wire = mek.to_wire_bytes();

        let wrapped = wrap_mek(
            &sender_key,
            &recipient_key.verifying_key().to_bytes(),
            &wire,
        )
        .unwrap();
        assert_eq!(wrapped.len(), 68);

        let unwrapped = unwrap_mek(
            &recipient_key,
            &sender_key.verifying_key().to_bytes(),
            &wrapped,
        )
        .unwrap();
        assert_eq!(unwrapped, wire);

        let restored = MediaEncryptionKey::from_wire_bytes(&unwrapped).unwrap();
        assert_eq!(restored.generation(), 42);
        assert_eq!(restored.as_bytes(), mek.as_bytes());
    }

    #[test]
    fn wrong_recipient_fails() {
        let sender = derive_community_pseudonym(&[1u8; 32], "c");
        let recipient = derive_community_pseudonym(&[2u8; 32], "c");
        let wrong = derive_community_pseudonym(&[3u8; 32], "c");

        let mek = MediaEncryptionKey::generate(1);
        let wrapped = wrap_mek(
            &sender,
            &recipient.verifying_key().to_bytes(),
            &mek.to_wire_bytes(),
        )
        .unwrap();

        assert!(unwrap_mek(&wrong, &sender.verifying_key().to_bytes(), &wrapped,).is_err());
    }

    #[test]
    fn wrong_sender_key_fails() {
        let sender = derive_community_pseudonym(&[1u8; 32], "c");
        let fake_sender = derive_community_pseudonym(&[99u8; 32], "c");
        let recipient = derive_community_pseudonym(&[2u8; 32], "c");

        let mek = MediaEncryptionKey::generate(1);
        let wrapped = wrap_mek(
            &sender,
            &recipient.verifying_key().to_bytes(),
            &mek.to_wire_bytes(),
        )
        .unwrap();

        assert!(unwrap_mek(
            &recipient,
            &fake_sender.verifying_key().to_bytes(),
            &wrapped,
        )
        .is_err());
    }

    #[test]
    fn wrapped_too_short() {
        let sender = derive_community_pseudonym(&[1u8; 32], "c");
        let recipient = derive_community_pseudonym(&[2u8; 32], "c");
        assert!(unwrap_mek(&recipient, &sender.verifying_key().to_bytes(), &[0u8; 11],).is_err());
    }

    #[test]
    fn cross_community_fails() {
        let sender_a = derive_community_pseudonym(&[1u8; 32], "community_a");
        let recipient_a = derive_community_pseudonym(&[2u8; 32], "community_a");
        let recipient_b = derive_community_pseudonym(&[2u8; 32], "community_b");
        let sender_b = derive_community_pseudonym(&[1u8; 32], "community_b");

        let mek = MediaEncryptionKey::generate(1);
        let wrapped = wrap_mek(
            &sender_a,
            &recipient_a.verifying_key().to_bytes(),
            &mek.to_wire_bytes(),
        )
        .unwrap();

        assert!(unwrap_mek(&recipient_b, &sender_b.verifying_key().to_bytes(), &wrapped,).is_err());
    }
}
