//! Invite secret encryption for self-service community joins.
//!
//! Encrypts community secrets (slot_seed, MEK, subkey_index) into an invite
//! record using HKDF(invite_code) → AES-256-GCM. Only someone with the raw
//! invite code can decrypt the secrets.

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use rand::RngCore;
use sha2::{Digest, Sha256};

use crate::error::CryptoError;

/// HKDF info label for invite secret encryption key derivation.
const INVITE_HKDF_INFO: &[u8] = b"rekindle-invite-secrets-v1";

/// Derive an AES-256-GCM key from an invite code using HKDF-SHA256.
fn derive_invite_key(invite_code: &[u8]) -> [u8; 32] {
    let hkdf = hkdf::Hkdf::<Sha256>::new(None, invite_code);
    let mut key = [0u8; 32];
    hkdf.expand(INVITE_HKDF_INFO, &mut key)
        .expect("32-byte output is valid for HKDF-SHA256");
    key
}

/// Compute SHA-256 hash of an invite code (hex-encoded).
///
/// Used to store invite entries in the DHT manifest without exposing the
/// raw code. The joiner hashes their code to find the matching entry.
pub fn hash_invite_code(invite_code: &str) -> String {
    let hash = Sha256::digest(invite_code.as_bytes());
    hex::encode(hash)
}

/// Encrypt invite secrets with HKDF(invite_code) → AES-256-GCM.
///
/// Output format: `[12-byte nonce || ciphertext + 16-byte tag]`.
pub fn encrypt_invite_secrets(
    invite_code: &str,
    plaintext: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    let key = derive_invite_key(invite_code.as_bytes());
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|e| CryptoError::EncryptionError(e.to_string()))?;

    let mut nonce_bytes = [0u8; 12];
    rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| CryptoError::EncryptionError(e.to_string()))?;

    let mut output = Vec::with_capacity(12 + ciphertext.len());
    output.extend_from_slice(&nonce_bytes);
    output.extend_from_slice(&ciphertext);
    Ok(output)
}

/// Decrypt invite secrets with HKDF(invite_code) → AES-256-GCM.
///
/// Input format: `[12-byte nonce || ciphertext + 16-byte tag]`.
pub fn decrypt_invite_secrets(
    invite_code: &str,
    encrypted: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    if encrypted.len() < 12 {
        return Err(CryptoError::DecryptionError(
            "encrypted invite data too short".into(),
        ));
    }

    let key = derive_invite_key(invite_code.as_bytes());
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|e| CryptoError::DecryptionError(e.to_string()))?;

    let nonce = Nonce::from_slice(&encrypted[..12]);
    let ciphertext = &encrypted[12..];

    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| CryptoError::DecryptionError(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let code = "a1b2c3d4e5f6a7b8a1b2c3d4e5f6a7b8";
        let secrets = b"slot_seed:abc,mek:xyz,index:42";

        let encrypted = encrypt_invite_secrets(code, secrets).unwrap();
        let decrypted = decrypt_invite_secrets(code, &encrypted).unwrap();

        assert_eq!(secrets.as_slice(), &decrypted);
    }

    #[test]
    fn wrong_code_fails() {
        let code = "a1b2c3d4e5f6a7b8a1b2c3d4e5f6a7b8";
        let wrong = "ffffffffffffffffffffffffffffffff";
        let secrets = b"secret data";

        let encrypted = encrypt_invite_secrets(code, secrets).unwrap();
        assert!(decrypt_invite_secrets(wrong, &encrypted).is_err());
    }

    #[test]
    fn hash_is_deterministic() {
        let code = "testcode123";
        assert_eq!(hash_invite_code(code), hash_invite_code(code));
    }

    #[test]
    fn hash_differs_for_different_codes() {
        assert_ne!(hash_invite_code("code1"), hash_invite_code("code2"));
    }

    #[test]
    fn too_short_data_fails() {
        assert!(decrypt_invite_secrets("code", &[0u8; 11]).is_err());
    }
}
