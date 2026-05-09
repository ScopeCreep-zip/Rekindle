//! Ed25519 signing and verification for prekey bundles.
//!
//! Used by PQXDH to sign SPK, OPK, PQPK (one-time), and PQPK (last-resort)
//! with distinct algorithm-byte prefixes and domain-separation tags.
//!
//! Algorithm-byte scheme (PQXDH rev 2, F3 mitigation):
//! - `0x01` = X25519 key (SPK, OPK)
//! - `0x02` = ML-KEM-768 key (PQPK)
//!
//! Domain tags (F4 mitigation):
//! - `"OT"` for one-time PQ prekeys
//! - `"LR"` for last-resort PQ prekeys

use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair, UnparsedPublicKey, ED25519};

use crate::error::RatchetError;

/// Algorithm-byte prefix for X25519 keys.
pub const ALG_X25519: u8 = 0x01;
/// Algorithm-byte prefix for ML-KEM-768 keys.
pub const ALG_MLKEM768: u8 = 0x02;

/// Domain tag for one-time PQ prekeys.
pub const DOMAIN_OT: &[u8] = b"OT";
/// Domain tag for last-resort PQ prekeys.
pub const DOMAIN_LR: &[u8] = b"LR";

/// Generate an Ed25519 keypair from a 32-byte seed.
pub fn keypair_from_seed(seed: &[u8; 32]) -> Result<Ed25519KeyPair, RatchetError> {
    // PKCS#8 v2 wrapping is required by aws-lc-rs for Ed25519.
    // `from_seed_unchecked` accepts a raw 32-byte seed.
    Ed25519KeyPair::from_seed_unchecked(seed).map_err(|_| RatchetError::SignFailed)
}

/// Sign an X25519 prekey: `sign(0x01 || key_bytes)`.
pub fn sign_ec_prekey(sk: &Ed25519KeyPair, key_bytes: &[u8]) -> [u8; 64] {
    let mut msg = Vec::with_capacity(1 + key_bytes.len());
    msg.push(ALG_X25519);
    msg.extend_from_slice(key_bytes);
    let sig = sk.sign(&msg);
    let mut out = [0u8; 64];
    out.copy_from_slice(sig.as_ref());
    out
}

/// Sign a PQ prekey: `sign(0x02 || domain_tag || key_bytes)`.
///
/// `domain_tag` is `DOMAIN_OT` for one-time or `DOMAIN_LR` for last-resort.
pub fn sign_pq_prekey(sk: &Ed25519KeyPair, domain_tag: &[u8], key_bytes: &[u8]) -> [u8; 64] {
    let mut msg = Vec::with_capacity(1 + domain_tag.len() + key_bytes.len());
    msg.push(ALG_MLKEM768);
    msg.extend_from_slice(domain_tag);
    msg.extend_from_slice(key_bytes);
    let sig = sk.sign(&msg);
    let mut out = [0u8; 64];
    out.copy_from_slice(sig.as_ref());
    out
}

/// Verify an X25519 prekey signature.
pub fn verify_ec_prekey(
    vk: &[u8; 32],
    key_bytes: &[u8],
    signature: &[u8],
) -> Result<(), RatchetError> {
    let mut msg = Vec::with_capacity(1 + key_bytes.len());
    msg.push(ALG_X25519);
    msg.extend_from_slice(key_bytes);
    UnparsedPublicKey::new(&ED25519, vk)
        .verify(&msg, signature)
        .map_err(|_| RatchetError::PqxdhSigInvalid)
}

/// Verify a PQ prekey signature with domain tag.
pub fn verify_pq_prekey(
    vk: &[u8; 32],
    domain_tag: &[u8],
    key_bytes: &[u8],
    signature: &[u8],
) -> Result<(), RatchetError> {
    let mut msg = Vec::with_capacity(1 + domain_tag.len() + key_bytes.len());
    msg.push(ALG_MLKEM768);
    msg.extend_from_slice(domain_tag);
    msg.extend_from_slice(key_bytes);
    UnparsedPublicKey::new(&ED25519, vk)
        .verify(&msg, signature)
        .map_err(|_| RatchetError::PqxdhSigInvalid)
}

/// Extract the 32-byte Ed25519 public key from a keypair.
pub fn public_key_bytes(kp: &Ed25519KeyPair) -> [u8; 32] {
    let pk = kp.public_key();
    let mut out = [0u8; 32];
    out.copy_from_slice(pk.as_ref());
    out
}
