use rekindle_codec::governance_op::genesis::{
    decode_genesis, encode_genesis, genesis_unsigned_bytes, SignedGenesis,
};
use rekindle_governance::genesis::{genesis_signing_bytes, verify_genesis, GenesisRejection};
use rekindle_secrets::ed25519_dalek::{Signer, SigningKey};
use rekindle_types::governance::{AdmissionMode, GenesisCert};
use rekindle_types::id::PseudonymKey;

const ANCHOR: &str = "VLD0:anchor";

/// A cert with two further creators and a predecessor, signed by both.
fn signed() -> (SignedGenesis, SigningKey) {
    let k_a = SigningKey::from_bytes(&[7; 32]);
    let creator = SigningKey::from_bytes(&[9; 32]);
    let mut cert: GenesisCert = rekindle_governance::fixture::cert(
        &PseudonymKey(creator.verifying_key().to_bytes()),
        AdmissionMode::Open,
    );
    cert.additional_creators = vec![PseudonymKey([2; 32]), PseudonymKey([3; 32])];
    cert.predecessor = Some("VLD0:old".into());
    let anchor_owner = k_a.verifying_key().to_bytes();
    let message = genesis_signing_bytes(ANCHOR, &genesis_unsigned_bytes(&cert, &anchor_owner));
    let signed = SignedGenesis {
        anchor_sig: k_a.sign(&message).to_bytes(),
        creator_sig: creator.sign(&message).to_bytes(),
        cert,
        anchor_owner,
    };
    (signed, creator)
}

/// Re-encode `signed` (sigs kept), decode it as a reader would, verify.
fn verify_as_read(signed: &SignedGenesis) -> Result<(), GenesisRejection> {
    let decoded = decode_genesis(&encode_genesis(signed).unwrap()).unwrap();
    let s = decoded.signed;
    verify_genesis(
        ANCHOR,
        &decoded.unsigned,
        &s.anchor_owner,
        &s.anchor_sig,
        &s.cert.creator_pseudonym,
        &s.creator_sig,
    )
}

#[test]
fn a_tampered_creator_set_or_predecessor_fails_the_cert_signature() {
    let (signed, _) = signed();
    assert_eq!(verify_as_read(&signed), Ok(()));
    let mut added = signed.clone();
    added.cert.additional_creators.push(PseudonymKey([4; 32]));
    assert_eq!(
        verify_as_read(&added),
        Err(GenesisRejection::BadAnchorSignature)
    );
    let mut dropped = signed.clone();
    dropped.cert.additional_creators.remove(0);
    assert_eq!(
        verify_as_read(&dropped),
        Err(GenesisRejection::BadAnchorSignature)
    );
    let mut moved = signed.clone();
    moved.cert.predecessor = Some("VLD0:other".into());
    assert_eq!(
        verify_as_read(&moved),
        Err(GenesisRejection::BadAnchorSignature)
    );
    let mut unset = signed;
    unset.cert.predecessor = None;
    assert_eq!(
        verify_as_read(&unset),
        Err(GenesisRejection::BadAnchorSignature)
    );
}
