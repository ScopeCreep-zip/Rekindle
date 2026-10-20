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
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use rand::rngs::OsRng;
    use rekindle_secrets::pq_keys::MlKemSecret;

    const AT: i64 = 1_760_000_000_000;
    const LR: PqRole = PqRole::LastResort { issued_at_ms: AT };

    fn curve() -> X25519Public {
        X25519Public::from([42u8; 32])
    }

    #[test]
    fn spk_and_opk_signatures_round_trip() {
        let signing = SigningKey::generate(&mut OsRng);
        let verifying = signing.verifying_key();
        let key = curve();
        let sig = signing.sign(&spk_signing_payload(7, AT, &key)).to_bytes();
        verify_spk(&verifying, 7, AT, key.as_bytes(), &sig).expect("valid SPK signature");
        let sig = signing.sign(&opk_signing_payload(9, &key)).to_bytes();
        verify_opk(&verifying, 9, key.as_bytes(), &sig).expect("valid OPK signature");
    }

    #[test]
    fn pq_signature_round_trips() {
        let signing = SigningKey::generate(&mut OsRng);
        let (_, pqpk) = MlKemSecret::generate();
        let sig = signing.sign(&pq_signing_payload(LR, 3, &pqpk)).to_bytes();
        verify_pq(&signing.verifying_key(), LR, 3, pqpk.as_bytes(), &sig)
            .expect("valid PQ-LR signature");
    }

    /// Each payload is its registered domain, then the id, the head's
    /// time where it has one, then the encoded key.
    #[test]
    fn each_payload_leads_with_its_registered_domain() {
        let key = curve();
        let expect = |domain: &str, at: Option<i64>, encoded: &[u8]| {
            let mut out = domain.as_bytes().to_vec();
            out.extend_from_slice(&7u32.to_be_bytes());
            if let Some(at) = at {
                out.extend_from_slice(&at.to_be_bytes());
            }
            out.extend_from_slice(encoded);
            out
        };
        let ec = encode_ec(&key);
        assert_eq!(
            spk_signing_payload(7, AT, &key),
            expect(domains::PREKEY_SPK_SIG, Some(AT), &ec)
        );
        assert_eq!(
            opk_signing_payload(7, &key),
            expect(domains::PREKEY_OPK_SIG, None, &ec)
        );
        let (_, pqpk) = MlKemSecret::generate();
        let kem = encode_kem(&pqpk);
        assert_eq!(
            pq_signing_payload(LR, 7, &pqpk),
            expect(domains::PREKEY_PQ_LR_SIG, Some(AT), &kem)
        );
        assert_eq!(
            pq_signing_payload(PqRole::OneTime, 7, &pqpk),
            expect(domains::PREKEY_PQ_OT_SIG, None, &kem)
        );
    }

    /// A signature binds the role: an LR signature is not an OT one, and
    /// an SPK signature is not an OPK one over the same key and id.
    #[test]
    fn a_signature_does_not_verify_in_another_role() {
        let signing = SigningKey::generate(&mut OsRng);
        let verifying = signing.verifying_key();
        let (_, pqpk) = MlKemSecret::generate();
        let sig = signing.sign(&pq_signing_payload(LR, 3, &pqpk)).to_bytes();
        assert!(verify_pq(&verifying, PqRole::OneTime, 3, pqpk.as_bytes(), &sig).is_err());

        let key = curve();
        let sig = signing.sign(&spk_signing_payload(5, AT, &key)).to_bytes();
        assert!(verify_opk(&verifying, 5, key.as_bytes(), &sig).is_err());
    }

    /// The id and the head's time are bound: the same key under another
    /// id, or the head re-dated, does not verify.
    #[test]
    fn a_signature_does_not_verify_under_another_id_or_time() {
        let signing = SigningKey::generate(&mut OsRng);
        let verifying = signing.verifying_key();
        let key = curve();
        let sig = signing.sign(&spk_signing_payload(5, AT, &key)).to_bytes();
        assert!(verify_spk(&verifying, 6, AT, key.as_bytes(), &sig).is_err());
        assert!(verify_spk(&verifying, 5, AT + 1, key.as_bytes(), &sig).is_err());
        let (_, pqpk) = MlKemSecret::generate();
        let sig = signing
            .sign(&pq_signing_payload(PqRole::OneTime, 1, &pqpk))
            .to_bytes();
        assert!(verify_pq(&verifying, PqRole::OneTime, 2, pqpk.as_bytes(), &sig).is_err());
        let sig = signing.sign(&pq_signing_payload(LR, 1, &pqpk)).to_bytes();
        let redated = PqRole::LastResort {
            issued_at_ms: AT + 1,
        };
        assert!(verify_pq(&verifying, redated, 1, pqpk.as_bytes(), &sig).is_err());
    }

    #[test]
    fn wrong_key_or_shape_rejects_signature() {
        let signing_a = SigningKey::generate(&mut OsRng);
        let verifying_b = SigningKey::generate(&mut OsRng).verifying_key();
        let key = curve();
        let sig = signing_a.sign(&spk_signing_payload(1, AT, &key)).to_bytes();
        assert!(verify_spk(&verifying_b, 1, AT, key.as_bytes(), &sig).is_err());
        assert!(verify_spk(&signing_a.verifying_key(), 1, AT, &[0; 31], &sig).is_err());
        assert!(verify_spk(
            &signing_a.verifying_key(),
            1,
            AT,
            key.as_bytes(),
            &sig[..63]
        )
        .is_err());
    }
}
