//! Key derivation functions — pseudonyms, SMPL slot keypairs and the
//! file-cache name key.
//!
//! Pseudonyms: HKDF-SHA256(master_secret, community_id) → Ed25519 keypair.
//! Slot keypairs: HKDF-SHA256(slot_seed, "rekindle-slot-{index}") → Ed25519 keypair.
//! File-cache names: HKDF-SHA256(salt none, master_secret, info
//! `FILE_CACHE_NAMES`) → 32-byte BLAKE3 key.

use ed25519_dalek::SigningKey;
use hkdf::Hkdf;
use rekindle_types::domains;
use rekindle_types::error::CryptoError;
use sha2::Sha256;
use x25519_dalek::StaticSecret;
use zeroize::Zeroizing;

/// Derive a unique, unlinkable Ed25519 keypair for a specific community.
///
/// Uses HKDF-SHA256 with a fixed salt. Deterministic: same inputs always
/// produce the same keypair. Different community IDs produce different
/// keypairs, making cross-community identity correlation impossible.
pub fn derive_community_pseudonym(master_secret: &[u8; 32], community_id: &str) -> SigningKey {
    let hkdf = Hkdf::<Sha256>::new(Some(domains::COMMUNITY_PSEUDONYM.as_bytes()), master_secret);
    let mut seed = [0u8; 32];
    hkdf.expand(community_id.as_bytes(), &mut seed)
        .expect("32-byte output is a valid HKDF-SHA256 length");
    SigningKey::from_bytes(&seed)
}

/// Owner keypair for this author's `page_index`-th governance overflow record.
///
/// Page 0 is the author's primary SMPL governance subkey; overflow records
/// begin at page 1. When one author's compacted governance entry log exceeds a
/// single subkey, the excess spills into a chain of member-owned overflow
/// records and the primary subkey carries an `overflow_next` pointer
/// (architecture §"Follow GovernanceOverflow pointers", line 1609).
///
/// The owner keypair is derived deterministically from the identity secret so
/// the overflow record is re-openable on any device with **zero persisted
/// keypair** — Veilid's `create_dht_record(.., owner: Some(kp))` is
/// deterministic for a given owner + schema, so re-deriving this keypair on a
/// fresh install yields the same record. Domain-separated from the pseudonym
/// and slot derivations by its own HKDF salt.
pub fn derive_governance_overflow_keypair(
    master_secret: &[u8; 32],
    community_id: &str,
    page_index: u32,
) -> SigningKey {
    let hkdf = Hkdf::<Sha256>::new(Some(domains::GOV_OVERFLOW.as_bytes()), master_secret);
    let mut seed = [0u8; 32];
    hkdf.expand(format!("{community_id}:{page_index}").as_bytes(), &mut seed)
        .expect("32-byte output is a valid HKDF-SHA256 length");
    SigningKey::from_bytes(&seed)
}

/// The key that names an identity's file-cache directories (plan E3.0a):
/// every directory under `state/{pk}.files/` is a keyed BLAKE3 hash under
/// it, so the names are stable across restarts and say nothing about the
/// communities, conversations or attachments they hold to anyone without
/// the identity secret. Domain-separated by its label in HKDF `info`
/// (RFC 5869 §3.2: `info` binds the output to its context; the salt is
/// for extraction and is omitted for an input that is already a uniform
/// key, §3.1).
pub fn derive_file_cache_name_key(master_secret: &[u8; 32]) -> Zeroizing<[u8; 32]> {
    let hkdf = Hkdf::<Sha256>::new(None, master_secret);
    let mut key = Zeroizing::new([0u8; 32]);
    hkdf.expand(domains::FILE_CACHE_NAMES.as_bytes(), key.as_mut())
        .expect("32-byte output is a valid HKDF-SHA256 length");
    key
}

/// Sign arbitrary bytes with a pseudonym key, returning a 64-byte Ed25519 signature.
pub fn sign_with_pseudonym(signing_key: &SigningKey, data: &[u8]) -> [u8; 64] {
    use ed25519_dalek::Signer;
    signing_key.sign(data).to_bytes()
}

/// Verify an Ed25519 signature against a pseudonym public key. Used by
/// receivers to authenticate that DHT-stored content (governance entries,
/// channel messages, presence) actually came from the claimed author —
/// the SMPL slot keypair signature alone proves only "some community
/// member wrote this," because the slot seed is community-shared (see
/// `derive_slot_keypair` below).
pub fn verify_pseudonym_signature(
    public_key_bytes: &[u8; 32],
    data: &[u8],
    signature: &[u8; 64],
) -> Result<(), CryptoError> {
    use ed25519_dalek::{Signature, VerifyingKey};
    let vk = VerifyingKey::from_bytes(public_key_bytes)
        .map_err(|e| CryptoError::Verification(format!("bad pseudonym public key: {e}")))?;
    let sig = Signature::from_bytes(signature);
    vk.verify_strict(data, &sig)
        .map_err(|e| CryptoError::Verification(format!("pseudonym signature: {e}")))
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
    let info = format!("{}{slot}", domains::SLOT_KEY_PREFIX);
    let mut okm = [0u8; 32];
    hk.expand(info.as_bytes(), &mut okm)
        .map_err(|_| CryptoError::KeyGeneration("HKDF expand failed for slot keypair".into()))?;
    Ok(SigningKey::from_bytes(&okm))
}

#[cfg(test)]
#[path = "derive_tests.rs"]
mod tests;
