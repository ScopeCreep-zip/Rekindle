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
