//! The prekey signatures (plan E2.4): each covers the prekey's id and its
//! `EncodeEC`/`EncodeKEM` encoding (PQXDH rev. 3 §2.3) under its own
//! registered domain (`rekindle_types::domains`), so a signature binds one
//! key to one id in one role, and no other identity-key signature can be
//! presented as one:
//! - signed prekey: `PREKEY_SPK_SIG ‖ u32be(spk_id) ‖ i64be(issued_at) ‖
//!   EncodeEC(spk)`;
//! - ML-KEM last-resort prekey: `PREKEY_PQ_LR_SIG ‖ u32be(id) ‖
//!   i64be(issued_at) ‖ EncodeKEM(pqpk)`;
//! - one-time ML-KEM prekey: `PREKEY_PQ_OT_SIG ‖ u32be(id) ‖ EncodeKEM(pqpk)`;
//! - one-time curve prekey: `PREKEY_OPK_SIG ‖ u32be(opk_id) ‖ EncodeEC(opk)`.
//!
//! The head's two keys sign its `issued_at`, so a reader can enforce the
//! head's age (`prekey_record::SIGNED_MAX_AGE_MS`) on a value only the
//! identity could have written.

use ed25519_dalek::{Signature, VerifyingKey};
use rekindle_secrets::pq_keys::MlKemPublic;
use rekindle_types::domains;
use x25519_dalek::PublicKey as X25519Public;

use super::encode::{encode_ec, encode_kem};
use super::keys::{ml_kem_public_from_bytes, x25519_from_bytes};
use super::PqxdhError;

/// The role of an ML-KEM prekey, which its signature binds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PqRole {
    /// The last-resort key, signed with the head's `issued_at`.
    LastResort { issued_at_ms: i64 },
    /// A one-time key.
    OneTime,
}

/// The bytes the signed prekey's signature covers.
#[must_use]
pub fn spk_signing_payload(spk_id: u32, issued_at_ms: i64, spk: &X25519Public) -> Vec<u8> {
    payload(
        domains::PREKEY_SPK_SIG,
        spk_id,
        Some(issued_at_ms),
        &encode_ec(spk),
    )
}

/// The bytes an ML-KEM prekey's signature covers.
#[must_use]
pub fn pq_signing_payload(role: PqRole, id: u32, pqpk: &MlKemPublic) -> Vec<u8> {
    let kem = encode_kem(pqpk);
    match role {
        PqRole::LastResort { issued_at_ms } => {
            payload(domains::PREKEY_PQ_LR_SIG, id, Some(issued_at_ms), &kem)
        }
        PqRole::OneTime => payload(domains::PREKEY_PQ_OT_SIG, id, None, &kem),
    }
}

/// The bytes a one-time curve prekey's signature covers.
#[must_use]
pub fn opk_signing_payload(opk_id: u32, opk: &X25519Public) -> Vec<u8> {
    payload(domains::PREKEY_OPK_SIG, opk_id, None, &encode_ec(opk))
}

/// Verify the signed prekey's signature under `identity_key`.
pub fn verify_spk(
    identity_key: &VerifyingKey,
    spk_id: u32,
    issued_at_ms: i64,
    spk_bytes: &[u8],
    signature_bytes: &[u8],
) -> Result<(), PqxdhError> {
    let payload = spk_signing_payload(spk_id, issued_at_ms, &x25519_from_bytes(spk_bytes)?);
    verify(identity_key, &payload, signature_bytes, "SPK")
}

/// Verify an ML-KEM prekey's signature under `identity_key`. The role is
/// bound, so a last-resort signature never verifies as a one-time one.
pub fn verify_pq(
    identity_key: &VerifyingKey,
    role: PqRole,
    id: u32,
    pqpk_bytes: &[u8],
    signature_bytes: &[u8],
) -> Result<(), PqxdhError> {
    let payload = pq_signing_payload(role, id, &ml_kem_public_from_bytes(pqpk_bytes)?);
    let what = match role {
        PqRole::LastResort { .. } => "PQ LR",
        PqRole::OneTime => "PQ OT",
    };
    verify(identity_key, &payload, signature_bytes, what)
}

/// Verify a one-time curve prekey's signature under `identity_key`.
pub fn verify_opk(
    identity_key: &VerifyingKey,
    opk_id: u32,
    opk_bytes: &[u8],
    signature_bytes: &[u8],
) -> Result<(), PqxdhError> {
    let payload = opk_signing_payload(opk_id, &x25519_from_bytes(opk_bytes)?);
    verify(identity_key, &payload, signature_bytes, "OPK")
}

fn payload(domain: &str, id: u32, issued_at_ms: Option<i64>, encoded: &[u8]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(domain.len() + 4 + 8 + encoded.len());
    payload.extend_from_slice(domain.as_bytes());
    payload.extend_from_slice(&id.to_be_bytes());
    if let Some(at) = issued_at_ms {
        payload.extend_from_slice(&at.to_be_bytes());
    }
    payload.extend_from_slice(encoded);
    payload
}

fn verify(
    identity_key: &VerifyingKey,
    payload: &[u8],
    signature_bytes: &[u8],
    what: &str,
) -> Result<(), PqxdhError> {
    let sig = parse_signature(signature_bytes)?;
    identity_key
        .verify_strict(payload, &sig)
        .map_err(|e| PqxdhError::SignatureVerify(format!("{what}: {e}")))
}

fn parse_signature(bytes: &[u8]) -> Result<Signature, PqxdhError> {
    if bytes.len() != Signature::BYTE_SIZE {
        return Err(PqxdhError::SignatureVerify(format!(
            "signature length {} (expected {})",
            bytes.len(),
            Signature::BYTE_SIZE
        )));
    }
    let mut arr = [0u8; Signature::BYTE_SIZE];
    arr.copy_from_slice(bytes);
    Ok(Signature::from_bytes(&arr))
}

#[cfg(test)]
#[path = "verify_tests.rs"]
mod tests;
