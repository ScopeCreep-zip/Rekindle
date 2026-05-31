//! Ed25519 signature verification for prekey bundle fields.

use ed25519_dalek::{Signature, Verifier, VerifyingKey};

use super::PqxdhError;

const SPK_DOMAIN_TAG: u8 = 0x01;
const PQ_DOMAIN_TAG: u8 = 0x02;

/// Verify the signed-prekey signature: `Ed25519(IK_B, 0x01 || SPK_B)`.
pub fn verify_spk(
    identity_key: &VerifyingKey,
    spk_bytes: &[u8],
    signature_bytes: &[u8],
) -> Result<(), PqxdhError> {
    let sig = parse_signature(signature_bytes)?;
    let mut payload = Vec::with_capacity(1 + spk_bytes.len());
    payload.push(SPK_DOMAIN_TAG);
    payload.extend_from_slice(spk_bytes);
    identity_key
        .verify(&payload, &sig)
        .map_err(|e| PqxdhError::SignatureVerify(format!("SPK: {e}")))
}

/// Verify a PQ prekey signature: `Ed25519(IK_B, 0x02 || domain_subtag || pqpk)`.
///
/// `domain_subtag` is `b"LR"` for the last-resort key and `b"OT"` for the
/// one-time key. The domain separation prevents an attacker from binding
/// a one-time signature to a last-resort context (or vice versa).
pub fn verify_pq(
    identity_key: &VerifyingKey,
    domain_subtag: &[u8],
    pqpk_bytes: &[u8],
    signature_bytes: &[u8],
) -> Result<(), PqxdhError> {
    let sig = parse_signature(signature_bytes)?;
    let mut payload = Vec::with_capacity(1 + domain_subtag.len() + pqpk_bytes.len());
    payload.push(PQ_DOMAIN_TAG);
    payload.extend_from_slice(domain_subtag);
    payload.extend_from_slice(pqpk_bytes);
    identity_key
        .verify(&payload, &sig)
        .map_err(|e| PqxdhError::SignatureVerify(format!("PQ {}: {e}", String::from_utf8_lossy(domain_subtag))))
}

/// Produce the bytes that `verify_spk` checks against — used by the
/// signer side to construct signatures with the same domain separation.
#[must_use]
pub fn spk_signing_payload(spk_bytes: &[u8]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(1 + spk_bytes.len());
    payload.push(SPK_DOMAIN_TAG);
    payload.extend_from_slice(spk_bytes);
    payload
}

/// Produce the bytes `verify_pq` checks against — used by the signer side.
#[must_use]
pub fn pq_signing_payload(domain_subtag: &[u8], pqpk_bytes: &[u8]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(1 + domain_subtag.len() + pqpk_bytes.len());
    payload.push(PQ_DOMAIN_TAG);
    payload.extend_from_slice(domain_subtag);
    payload.extend_from_slice(pqpk_bytes);
    payload
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

    #[test]
    fn spk_signature_round_trips() {
        let signing = SigningKey::generate(&mut OsRng);
        let verifying = signing.verifying_key();
        let spk = [42u8; 32];
        let sig = signing.sign(&spk_signing_payload(&spk));
        verify_spk(&verifying, &spk, &sig.to_bytes()).expect("valid SPK signature");
    }

    #[test]
    fn pq_signature_round_trips() {
        let signing = SigningKey::generate(&mut OsRng);
        let verifying = signing.verifying_key();
        let pqpk = vec![7u8; 1184];
        let sig = signing.sign(&pq_signing_payload(b"LR", &pqpk));
        verify_pq(&verifying, b"LR", &pqpk, &sig.to_bytes()).expect("valid PQ-LR signature");
    }

    #[test]
    fn pq_lr_signature_does_not_verify_as_ot() {
        // Domain separation: an LR signature must NOT verify as an OT signature.
        let signing = SigningKey::generate(&mut OsRng);
        let verifying = signing.verifying_key();
        let pqpk = vec![7u8; 1184];
        let sig = signing.sign(&pq_signing_payload(b"LR", &pqpk));
        assert!(verify_pq(&verifying, b"OT", &pqpk, &sig.to_bytes()).is_err());
    }

    #[test]
    fn wrong_key_rejects_signature() {
        let signing_a = SigningKey::generate(&mut OsRng);
        let verifying_b = SigningKey::generate(&mut OsRng).verifying_key();
        let spk = [0u8; 32];
        let sig = signing_a.sign(&spk_signing_payload(&spk));
        assert!(verify_spk(&verifying_b, &spk, &sig.to_bytes()).is_err());
    }
}
