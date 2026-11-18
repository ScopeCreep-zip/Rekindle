//! Invite secret encryption for self-sovereign community joins.
//!
//! Encrypts community secrets (slot_seed, MEK, registry_key) into an invite
//! blob using HKDF(invite_code) → AES-256-GCM. Only someone with the raw
//! invite code can decrypt.

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use ed25519_dalek::{Signer, SigningKey};
use hkdf::Hkdf;
use rand::RngCore;
use rekindle_types::domains;
use rekindle_types::error::CryptoError;
use sha2::Sha256;
use zeroize::Zeroizing;

/// Derive an AES-256-GCM key from an invite code using HKDF-SHA256.
fn derive_invite_key(invite_code: &[u8]) -> [u8; 32] {
    let hkdf = Hkdf::<Sha256>::new(None, invite_code);
    let mut key = [0u8; 32];
    hkdf.expand(domains::INVITE_SECRETS.as_bytes(), &mut key)
        .expect("32-byte output is valid for HKDF-SHA256");
    key
}

/// The invite's join key: the Ed25519 key whose seed is `blake3::derive_key`
/// under [`domains::INVITE_JOIN`] over the code's bytes (plan E3.2 S10).
/// Only a holder of the code can make it.
fn invite_join_key(invite_code: &str) -> SigningKey {
    let seed = Zeroizing::new(blake3::derive_key(
        domains::INVITE_JOIN,
        invite_code.as_bytes(),
    ));
    SigningKey::from_bytes(&seed)
}

/// An invite's public id, `InviteCreated.code_hash`: the hex of the public
/// half of [`invite_join_key`] (plan E3.2 S10). It is both the lookup id
/// invite entries are stored under (the raw code never is) and the key
/// every reader verifies a joiner's invite proof against without the code
/// ([`sign_invite_join`]); readers never hold the code.
#[must_use]
pub fn hash_invite_code(invite_code: &str) -> String {
    hex::encode(invite_join_key(invite_code).verifying_key().to_bytes())
}

/// A joiner's invite proof (plan E3.2 S10): Ed25519 by the invite's join
/// key over `signed`, the proof bytes `rekindle_governance::op::
/// invite_proof_bytes` lays out (community, joiner pseudonym, member record,
/// [`hash_invite_code`]). It verifies under the public `code_hash`, so a
/// reader of the op log checks it, but only a holder of the code makes it,
/// and it names its joiner, so it cannot be replayed for another.
#[must_use]
pub fn sign_invite_join(invite_code: &str, signed: &[u8]) -> [u8; 64] {
    invite_join_key(invite_code).sign(signed).to_bytes()
}

/// The tag a joiner puts on a pre-join checkpoint hint or snapshot request
/// (plan E3.2 S7c.8 (2)): BLAKE3 keyed under `derive_key` of
/// [`domains::INVITE_READ_TAG`] over the invite code, over `signed` (the
/// request's signed bytes, which name the community, what is asked and the
/// code's lookup id). The op log carries only that lookup id, so a party
/// that saw the `InviteCreated` but never the code cannot tag a request;
/// the responding inviter checks the tag under its own copy of the code
/// ([`pre_join_read_tag_verifies`]).
#[must_use]
pub fn pre_join_read_tag(invite_code: &str, signed: &[u8]) -> [u8; 32] {
    let key = blake3::derive_key(domains::INVITE_READ_TAG, invite_code.as_bytes());
    *blake3::keyed_hash(&key, signed).as_bytes()
}

/// Whether `tag` is [`pre_join_read_tag`] of `signed` under `invite_code`,
/// compared in constant time (`blake3::Hash`'s equality).
#[must_use]
pub fn pre_join_read_tag_verifies(invite_code: &str, signed: &[u8], tag: &[u8; 32]) -> bool {
    blake3::Hash::from(pre_join_read_tag(invite_code, signed)) == blake3::Hash::from(*tag)
}

/// Whether `sig` is a [`sign_invite_join`] proof over `signed` by the join
/// key whose public half is `code_hash` (64 hex digits): what every reader
/// checks, without the code. A `code_hash` that is not such a key verifies
/// nothing.
#[must_use]
pub fn invite_join_verifies(code_hash: &str, signed: &[u8], sig: &[u8; 64]) -> bool {
    let mut key = [0u8; 32];
    hex::decode_to_slice(code_hash, &mut key).is_ok()
        && crate::sign::verify_signature(&key, signed, sig).is_ok()
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

    /// The id is the public half of the Ed25519 key seeded by BLAKE3
    /// `derive_key` under the invite-join context: a proof by that key
    /// verifies under the id, and neither the id nor a bare hash of the code
    /// makes one.
    #[test]
    fn the_id_is_the_join_key_and_only_the_code_signs_under_it() {
        use crate::sign::verify_signature;
        let code = "a1b2c3d4e5f6a7b8a1b2c3d4e5f6a7b8";
        let id = hash_invite_code(code);
        let seed = blake3::derive_key(domains::INVITE_JOIN, code.as_bytes());
        let public = SigningKey::from_bytes(&seed).verifying_key().to_bytes();
        assert_eq!(id, hex::encode(public));
        let sig = sign_invite_join(code, b"proof");
        assert!(verify_signature(&public, b"proof", &sig).is_ok());
        assert!(invite_join_verifies(&id, b"proof", &sig));
        assert!(!invite_join_verifies(&id, b"other", &sig));
        assert!(!invite_join_verifies("not hex", b"proof", &sig));
        assert!(verify_signature(&public, b"other", &sig).is_err());
        let by_id = sign_invite_join(&id, b"proof");
        assert!(verify_signature(&public, b"proof", &by_id).is_err());
        assert_ne!(id, hex::encode(seed));
    }

    /// The tag is keyed by the code, not its public lookup id: the id
    /// cannot produce it, and it binds the bytes it covers.
    #[test]
    fn a_pre_join_read_tag_needs_the_code() {
        let code = "a1b2c3d4e5f6a7b8a1b2c3d4e5f6a7b8";
        let tag = pre_join_read_tag(code, b"request");
        assert!(pre_join_read_tag_verifies(code, b"request", &tag));
        assert!(!pre_join_read_tag_verifies(code, b"other", &tag));
        assert!(!pre_join_read_tag_verifies(
            &hash_invite_code(code),
            b"request",
            &tag
        ));
        let id_tag = pre_join_read_tag(&hash_invite_code(code), b"request");
        assert!(!pre_join_read_tag_verifies(code, b"request", &id_tag));
    }

    #[test]
    fn too_short_fails() {
        assert!(decrypt_invite_secrets("code", &[0u8; 11]).is_err());
    }
}
