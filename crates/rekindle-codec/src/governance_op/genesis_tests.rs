use super::*;

fn signed() -> SignedGenesis {
    SignedGenesis {
        cert: GenesisCert {
            name: "c".into(),
            policy: AdmissionMode::ApprovalRequired,
            initial_roles: vec![RoleId([0; 16]), RoleId([0xFF; 16])],
            creator_pseudonym: PseudonymKey([1; 32]),
            creator_member_record: "VLD0:m".into(),
            creator_member_record_owner: "VLD0:o".into(),
            created_at: 7,
            additional_creators: vec![PseudonymKey([5; 32]), PseudonymKey([6; 32])],
            predecessor: Some("VLD0:old".into()),
            creator_device: None,
        },
        anchor_owner: [2; 32],
        anchor_sig: [3; 64],
        creator_sig: [4; 64],
    }
}

#[test]
fn round_trips_with_the_unsigned_form() {
    let s = signed();
    let decoded = decode_genesis(&encode_genesis(&s).unwrap()).unwrap();
    assert_eq!(decoded.signed, s);
    assert_eq!(
        decoded.unsigned,
        genesis_unsigned_bytes(&s.cert, &s.anchor_owner)
    );
}

#[test]
fn a_short_signature_is_refused() {
    let mut builder = capnp::message::Builder::new_default();
    let mut b = builder.init_root::<genesis_cert::Builder<'_>>();
    b.set_creator_pseudonym(&[1; 32]);
    b.set_anchor_owner(&[2; 32]);
    b.set_anchor_sig(&[3; 10]);
    b.set_creator_sig(&[4; 64]);
    assert!(decode_genesis(&pack(&builder).unwrap()).is_err());
}

#[test]
fn a_ninth_additional_creator_is_refused() {
    let mut s = signed();
    s.cert.additional_creators = (0..=MAX_ADDITIONAL_CREATORS)
        .map(|i| PseudonymKey([u8::try_from(i).unwrap() + 10; 32]))
        .collect();
    assert!(encode_genesis(&s).is_err());
    // A writer that skips the bound is refused at decode.
    let mut builder = capnp::message::Builder::new_default();
    let mut b = builder.init_root::<genesis_cert::Builder<'_>>();
    b.set_creator_pseudonym(&[1; 32]);
    b.set_anchor_owner(&[2; 32]);
    b.set_anchor_sig(&[3; 64]);
    b.set_creator_sig(&[4; 64]);
    let mut list = b.init_additional_creators(9);
    for i in 0..9 {
        list.set(i, &[u8::try_from(i).unwrap() + 10; 32]);
    }
    assert!(decode_genesis(&pack(&builder).unwrap()).is_err());
    s.cert.additional_creators.pop();
    assert!(decode_genesis(&encode_genesis(&s).unwrap()).is_ok());
}

#[test]
fn the_unsigned_form_covers_the_creator_set_and_predecessor() {
    let s = signed();
    let base = genesis_unsigned_bytes(&s.cert, &s.anchor_owner);
    let mut other = s.cert.clone();
    other.additional_creators.pop();
    assert_ne!(genesis_unsigned_bytes(&other, &s.anchor_owner), base);
    let mut other = s.cert.clone();
    other.predecessor = None;
    assert_ne!(genesis_unsigned_bytes(&other, &s.anchor_owner), base);
}

#[test]
fn too_many_roles_are_refused() {
    let mut s = signed();
    s.cert.initial_roles = vec![RoleId([0; 16]); GENESIS_MAX_ROLES + 1];
    assert!(encode_genesis(&s).is_err());
}
