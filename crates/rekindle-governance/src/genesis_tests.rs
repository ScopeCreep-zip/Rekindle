use super::*;
use rekindle_secrets::ed25519_dalek::{Signer, SigningKey};

fn cert_with(additional: &[u8]) -> GenesisCert {
    let mut cert = crate::fixture::cert(
        &PseudonymKey([1; 32]),
        rekindle_types::governance::AdmissionMode::Open,
    );
    cert.additional_creators = additional.iter().map(|b| PseudonymKey([*b; 32])).collect();
    cert
}

#[test]
fn a_creator_set_with_a_repeat_or_a_ninth_is_refused() {
    assert_eq!(check_creator_set(&cert_with(&[2, 3])), Ok(()));
    assert_eq!(
        check_creator_set(&cert_with(&[2, 3, 4, 5, 6, 7, 8, 9])),
        Ok(())
    );
    assert_eq!(
        check_creator_set(&cert_with(&[2, 3, 4, 5, 6, 7, 8, 9, 10])),
        Err(GenesisRejection::TooManyCreators)
    );
    assert_eq!(
        check_creator_set(&cert_with(&[2, 3, 2])),
        Err(GenesisRejection::RepeatedCreator)
    );
    assert_eq!(
        check_creator_set(&cert_with(&[2, 1])),
        Err(GenesisRejection::RepeatedCreator)
    );
}

#[test]
fn both_signers_must_sign_the_same_bytes() {
    let k_a = SigningKey::from_bytes(&[7; 32]);
    let creator = SigningKey::from_bytes(&[9; 32]);
    let msg = genesis_signing_bytes("VLD0:a", b"cert");
    let a_sig = k_a.sign(&msg).to_bytes();
    let c_sig = creator.sign(&msg).to_bytes();
    let a_pub = k_a.verifying_key().to_bytes();
    let c_pub = PseudonymKey(creator.verifying_key().to_bytes());
    assert_eq!(
        verify_genesis("VLD0:a", b"cert", &a_pub, &a_sig, &c_pub, &c_sig),
        Ok(())
    );
    assert_eq!(
        verify_genesis("VLD0:b", b"cert", &a_pub, &a_sig, &c_pub, &c_sig),
        Err(GenesisRejection::BadAnchorSignature)
    );
    assert_eq!(
        verify_genesis("VLD0:a", b"cert", &a_pub, &a_sig, &c_pub, &a_sig),
        Err(GenesisRejection::BadCreatorSignature)
    );
}

/// Decision (16): an acceptance verifies only for its own anchor and
/// pseudonym, under the creator-accept label.
#[test]
fn a_creator_accept_binds_its_anchor_and_pseudonym() {
    let key = SigningKey::from_bytes(&[9; 32]);
    let pseudo = PseudonymKey(key.verifying_key().to_bytes());
    let sig = key
        .sign(&creator_accept_signing_bytes("VLD0:a", &pseudo))
        .to_bytes();
    assert!(verify_creator_accept("VLD0:a", &pseudo, &sig));
    assert!(!verify_creator_accept("VLD0:b", &pseudo, &sig));
    let other = PseudonymKey(SigningKey::from_bytes(&[8; 32]).verifying_key().to_bytes());
    assert!(!verify_creator_accept("VLD0:a", &other, &sig));
    let genesis_sig = key
        .sign(&genesis_signing_bytes("VLD0:a", &pseudo.0))
        .to_bytes();
    assert!(!verify_creator_accept("VLD0:a", &pseudo, &genesis_sig));
}
