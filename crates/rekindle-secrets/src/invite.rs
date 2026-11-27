//! Invite secret encryption for self-sovereign community joins.
//!
//! Encrypts community secrets (slot_seed, MEK, registry_key) into an invite
//! blob using HKDF(invite_code) → AES-256-GCM. Only someone with the raw
//! invite code can decrypt.

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
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

/// Whether `code_hash` is a public join key a proof can verify under
/// ([`invite_join_verifies`]): 64 lowercase hex digits (the form
/// [`hash_invite_code`] writes, which the join's lookup compares bytewise),
/// the canonical encoding of a curve point, not of small order (the strict
/// verification every reader runs refuses those keys). Readers refuse an
/// `InviteCreated` whose `code_hash` is not one (plan E3.2 S10).
#[must_use]
pub fn invite_join_key_valid(code_hash: &str) -> bool {
    let mut bytes = [0u8; 32];
    if hex::decode_to_slice(code_hash, &mut bytes).is_err() || hex::encode(bytes) != code_hash {
        return false;
    }
    VerifyingKey::from_bytes(&bytes)
        .is_ok_and(|key| !key.is_weak() && key.to_edwards().compress().to_bytes() == bytes)
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
#[path = "invite_tests.rs"]
mod tests;
