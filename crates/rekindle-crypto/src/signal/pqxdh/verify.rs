//! The prekey signatures (plan E2.4): each covers the prekey's id and its
//! `EncodeEC`/`EncodeKEM` encoding (PQXDH rev. 3 §2.3) under a domain tag,
//! so a signature binds one key to one id in one role:
//! - signed prekey: `0x01 ‖ u32be(spk_id) ‖ EncodeEC(spk)`;
//! - ML-KEM prekey: `0x02 ‖ "LR"|"OT" ‖ u32be(id) ‖ EncodeKEM(pqpk)`;
//! - one-time curve prekey: `0x03 ‖ u32be(opk_id) ‖ EncodeEC(opk)`.

use ed25519_dalek::{Signature, VerifyingKey};
use rekindle_secrets::pq_keys::MlKemPublic;
use x25519_dalek::PublicKey as X25519Public;

use super::encode::{encode_ec, encode_kem};
use super::keys::{ml_kem_public_from_bytes, x25519_from_bytes};
use super::PqxdhError;

const SPK_DOMAIN_TAG: u8 = 0x01;
const PQ_DOMAIN_TAG: u8 = 0x02;
const OPK_DOMAIN_TAG: u8 = 0x03;

/// The role of an ML-KEM prekey, which its signature binds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PqRole {
    /// The last-resort key (`"LR"`).
    LastResort,
    /// A one-time key (`"OT"`).
    OneTime,
}

impl PqRole {
    fn subtag(self) -> &'static [u8; 2] {
        match self {
            Self::LastResort => b"LR",
            Self::OneTime => b"OT",
        }
    }
}

/// The bytes the signed prekey's signature covers.
#[must_use]
pub fn spk_signing_payload(spk_id: u32, spk: &X25519Public) -> Vec<u8> {
    curve_payload(SPK_DOMAIN_TAG, spk_id, spk)
}

/// The bytes an ML-KEM prekey's signature covers.
#[must_use]
pub fn pq_signing_payload(role: PqRole, id: u32, pqpk: &MlKemPublic) -> Vec<u8> {
    let kem = encode_kem(pqpk);
    let mut payload = Vec::with_capacity(1 + 2 + 4 + kem.len());
    payload.push(PQ_DOMAIN_TAG);
    payload.extend_from_slice(role.subtag());
    payload.extend_from_slice(&id.to_be_bytes());
    payload.extend_from_slice(&kem);
    payload
}

/// The bytes a one-time curve prekey's signature covers.
#[must_use]
pub fn opk_signing_payload(opk_id: u32, opk: &X25519Public) -> Vec<u8> {
    curve_payload(OPK_DOMAIN_TAG, opk_id, opk)
}

/// Verify the signed prekey's signature under `identity_key`.
pub fn verify_spk(
    identity_key: &VerifyingKey,
    spk_id: u32,
    spk_bytes: &[u8],
    signature_bytes: &[u8],
) -> Result<(), PqxdhError> {
    let payload = spk_signing_payload(spk_id, &x25519_from_bytes(spk_bytes)?);
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
        PqRole::LastResort => "PQ LR",
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

fn curve_payload(tag: u8, id: u32, key: &X25519Public) -> Vec<u8> {
    let encoded = encode_ec(key);
    let mut payload = Vec::with_capacity(1 + 4 + encoded.len());
    payload.push(tag);
    payload.extend_from_slice(&id.to_be_bytes());
    payload.extend_from_slice(&encoded);
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

    fn curve() -> X25519Public {
        X25519Public::from([42u8; 32])
    }

    #[test]
    fn spk_and_opk_signatures_round_trip() {
        let signing = SigningKey::generate(&mut OsRng);
        let verifying = signing.verifying_key();
        let key = curve();
        let sig = signing.sign(&spk_signing_payload(7, &key)).to_bytes();
        verify_spk(&verifying, 7, key.as_bytes(), &sig).expect("valid SPK signature");
        let sig = signing.sign(&opk_signing_payload(9, &key)).to_bytes();
        verify_opk(&verifying, 9, key.as_bytes(), &sig).expect("valid OPK signature");
    }

    #[test]
    fn pq_signature_round_trips() {
        let signing = SigningKey::generate(&mut OsRng);
        let (_, pqpk) = MlKemSecret::generate();
        let sig = signing
            .sign(&pq_signing_payload(PqRole::LastResort, 3, &pqpk))
            .to_bytes();
        verify_pq(
            &signing.verifying_key(),
            PqRole::LastResort,
            3,
            pqpk.as_bytes(),
            &sig,
        )
        .expect("valid PQ-LR signature");
    }

    /// A signature binds the role: an LR signature is not an OT one, and
    /// an SPK signature is not an OPK one over the same key and id.
    #[test]
    fn a_signature_does_not_verify_in_another_role() {
        let signing = SigningKey::generate(&mut OsRng);
        let verifying = signing.verifying_key();
        let (_, pqpk) = MlKemSecret::generate();
        let sig = signing
            .sign(&pq_signing_payload(PqRole::LastResort, 3, &pqpk))
            .to_bytes();
        assert!(verify_pq(&verifying, PqRole::OneTime, 3, pqpk.as_bytes(), &sig).is_err());

        let key = curve();
        let sig = signing.sign(&spk_signing_payload(5, &key)).to_bytes();
        assert!(verify_opk(&verifying, 5, key.as_bytes(), &sig).is_err());
    }

    /// The id is bound: the same key under another id does not verify.
    #[test]
    fn a_signature_does_not_verify_under_another_id() {
        let signing = SigningKey::generate(&mut OsRng);
        let verifying = signing.verifying_key();
        let key = curve();
        let sig = signing.sign(&spk_signing_payload(5, &key)).to_bytes();
        assert!(verify_spk(&verifying, 6, key.as_bytes(), &sig).is_err());
        let (_, pqpk) = MlKemSecret::generate();
        let sig = signing
            .sign(&pq_signing_payload(PqRole::OneTime, 1, &pqpk))
            .to_bytes();
        assert!(verify_pq(&verifying, PqRole::OneTime, 2, pqpk.as_bytes(), &sig).is_err());
    }

    #[test]
    fn wrong_key_or_shape_rejects_signature() {
        let signing_a = SigningKey::generate(&mut OsRng);
        let verifying_b = SigningKey::generate(&mut OsRng).verifying_key();
        let key = curve();
        let sig = signing_a.sign(&spk_signing_payload(1, &key)).to_bytes();
        assert!(verify_spk(&verifying_b, 1, key.as_bytes(), &sig).is_err());
        assert!(verify_spk(&signing_a.verifying_key(), 1, &[0; 31], &sig).is_err());
        assert!(verify_spk(&signing_a.verifying_key(), 1, key.as_bytes(), &sig[..63]).is_err());
    }
}
