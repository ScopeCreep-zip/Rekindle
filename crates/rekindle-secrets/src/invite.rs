//! Invite secret encryption for self-sovereign community joins.
//!
//! Encrypts community secrets (slot_seed, MEK, registry_key) into an invite
//! blob using HKDF(invite_code) → AES-256-GCM. Only someone with the raw
//! invite code can decrypt.

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use hkdf::Hkdf;
use rand::RngCore;
use rekindle_types::domains;
use rekindle_types::error::CryptoError;
use sha2::Sha256;

/// Derive an AES-256-GCM key from an invite code using HKDF-SHA256.
fn derive_invite_key(invite_code: &[u8]) -> [u8; 32] {
    let hkdf = Hkdf::<Sha256>::new(None, invite_code);
    let mut key = [0u8; 32];
    hkdf.expand(domains::INVITE_SECRETS.as_bytes(), &mut key)
        .expect("32-byte output is valid for HKDF-SHA256");
    key
}

/// The lookup id of an invite code: `blake3::derive_key` under
/// [`domains::INVITE_CODE_ID`] over the code's bytes, hex-encoded
/// (crypto-primitives §7: identifier hashing is BLAKE3 with a context).
///
/// Invite entries are stored in the DHT under this id, so the raw code
/// never is.
#[must_use]
pub fn hash_invite_code(invite_code: &str) -> String {
    hex::encode(blake3::derive_key(
        domains::INVITE_CODE_ID,
        invite_code.as_bytes(),
    ))
}

/// Encrypt invite secrets with HKDF(invite_code) → AES-256-GCM.
///
/// Output: `[12-byte nonce || ciphertext + 16-byte tag]`.
pub fn encrypt_invite_secrets(invite_code: &str, plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let key = derive_invite_key(invite_code.as_bytes());
    let cipher =
        Aes256Gcm::new_from_slice(&key).map_err(|e| CryptoError::Encryption(e.to_string()))?;

    let mut nonce_bytes = [0u8; 12];
    rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| CryptoError::Encryption(e.to_string()))?;

    let mut output = Vec::with_capacity(12 + ciphertext.len());
    output.extend_from_slice(&nonce_bytes);
    output.extend_from_slice(&ciphertext);
    Ok(output)
}

/// Decrypt invite secrets with HKDF(invite_code) → AES-256-GCM.
///
/// Input: `[12-byte nonce || ciphertext + 16-byte tag]`.
pub fn decrypt_invite_secrets(invite_code: &str, encrypted: &[u8]) -> Result<Vec<u8>, CryptoError> {
    if encrypted.len() < 12 {
        return Err(CryptoError::Decryption(
            "encrypted invite data too short".into(),
        ));
    }

    let key = derive_invite_key(invite_code.as_bytes());
    let cipher =
        Aes256Gcm::new_from_slice(&key).map_err(|e| CryptoError::Decryption(e.to_string()))?;

    let nonce = Nonce::from_slice(&encrypted[..12]);
    cipher
        .decrypt(nonce, &encrypted[12..])
        .map_err(|e| CryptoError::Decryption(e.to_string()))
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
        let encrypted = encrypt_invite_secrets(code, b"secret").unwrap();
        assert!(decrypt_invite_secrets(wrong, &encrypted).is_err());
    }

    #[test]
    fn hash_deterministic() {
        let code = "testcode123";
        assert_eq!(hash_invite_code(code), hash_invite_code(code));
    }

    #[test]
    fn hash_differs() {
        assert_ne!(hash_invite_code("code1"), hash_invite_code("code2"));
    }

    /// The id is BLAKE3 `derive_key` under the invite-code context: not a
    /// bare BLAKE3 or SHA-256 of the code, and 32 bytes of hex.
    #[test]
    fn hash_is_blake3_under_the_invite_code_context() {
        use sha2::Digest;
        let code = "a1b2c3d4e5f6a7b8a1b2c3d4e5f6a7b8";
        let id = hash_invite_code(code);
        assert_eq!(
            id,
            hex::encode(blake3::derive_key(domains::INVITE_CODE_ID, code.as_bytes()))
        );
        assert_eq!(id.len(), 64);
        assert_ne!(id, blake3::hash(code.as_bytes()).to_hex().to_string());
        assert_ne!(id, hex::encode(Sha256::digest(code.as_bytes())));
    }

    #[test]
    fn too_short_fails() {
        assert!(decrypt_invite_secrets("code", &[0u8; 11]).is_err());
    }
}
