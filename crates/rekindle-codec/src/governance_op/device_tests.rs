//! The device-key wire (ADR 0022, plan E3.2 S7d-keys.1): the three arms at
//! their fresh ordinals, `MemberJoined`'s first device `@6`, the
//! `GenesisCert` creator's `@12`, round trips, bounds and signing coverage.

use rekindle_types::governance::{
    AdmissionMode, DeviceEndorsement, DeviceReadmit, DeviceRetired, DeviceSetElement,
    DeviceSetElementKind, GenesisCert, GovernanceEntry, GovernanceOp, MAX_DEVICE_SET_ELEMENTS,
};
use rekindle_types::id::PseudonymKey;

use super::genesis::genesis_unsigned_bytes;
use super::{decode_op, encode_op, op_unsigned_bytes};

pub(super) fn endorsement(b: u8) -> DeviceEndorsement {
    DeviceEndorsement {
        device_key: [b; 32],
        incarnation: 3,
        extends_set: [b.wrapping_add(1); 32],
        extends_elements: vec![
            DeviceSetElement::new(
                DeviceSetElementKind::Endorsement,
                [b.wrapping_add(2); 32],
                1,
            ),
            DeviceSetElement::new(DeviceSetElementKind::Retirement, [b.wrapping_add(2); 32], 1),
        ],
        device_record: format!("VLD0:{}", "d".repeat(43)),
        device_record_owner: format!("VLD0:{}", "o".repeat(43)),
        fork_evidence: [b.wrapping_add(3); 32],
        endorsement_sig: vec![b.wrapping_add(4); 64],
    }
}

pub(super) fn retired(b: u8) -> DeviceRetired {
    DeviceRetired {
        device_key: [b; 32],
        incarnation: 2,
        last_seq: u64::MAX - 1,
        retirement_sig: vec![b.wrapping_add(1); 64],
    }
}

pub(super) fn readmit(b: u8) -> DeviceReadmit {
    DeviceReadmit {
        member: PseudonymKey([b; 32]),
        member_record: format!("VLD0:{}", "m".repeat(43)),
        new_key: [b.wrapping_add(1); 32],
        incarnation: 4,
        fork_evidence: [b.wrapping_add(2); 32],
    }
}

fn op(body: GovernanceEntry) -> GovernanceOp {
    GovernanceOp {
        device_key: [0xA0; 32],
        author: PseudonymKey([0xA0; 32]),
        seq: 0,
        prev: None,
        deps: Vec::new(),
        issued_at: 1_700_000_000,
        body,
        sig: [0xC0; 64],
    }
}

fn joined(first_device: Option<DeviceEndorsement>) -> GovernanceEntry {
    GovernanceEntry::MemberJoined {
        member_record: "VLD0:m".into(),
        member_record_owner: "own".into(),
        invite_proof: None,
        owner_proof: vec![7; 64],
        first_device,
    }
}

/// The explicit ordinal of field `name` in the struct `T`.
fn ordinal<T: capnp::introspect::Introspect>(name: &str) -> u16 {
    let capnp::introspect::TypeVariant::Struct(raw) = T::introspect().which() else {
        panic!("not a struct");
    };
    let schema = capnp::schema::StructSchema::from(raw);
    let field = schema.find_field_by_name(name).unwrap().unwrap();
    match field.get_proto().get_ordinal().which().unwrap() {
        capnp::schema_capnp::field::ordinal::Explicit(n) => n,
        capnp::schema_capnp::field::ordinal::Implicit(()) => panic!("{name} has no ordinal"),
    }
}

#[test]
fn the_device_fields_sit_at_their_fresh_ordinals() {
    use crate::community_governance_capnp::{governance_entry, member_joined_payload};
    use crate::governance_op_capnp::genesis_cert;
    assert_eq!(ordinal::<governance_entry::Owned>("communitySuccessor"), 46);
    assert_eq!(ordinal::<governance_entry::Owned>("deviceKey"), 47);
    assert_eq!(ordinal::<governance_entry::Owned>("deviceRetired"), 48);
    assert_eq!(ordinal::<governance_entry::Owned>("deviceReadmit"), 49);
    assert_eq!(ordinal::<member_joined_payload::Owned>("ownerProof"), 5);
    assert_eq!(ordinal::<member_joined_payload::Owned>("firstDevice"), 6);
    assert_eq!(ordinal::<genesis_cert::Owned>("predecessor"), 11);
    assert_eq!(ordinal::<genesis_cert::Owned>("creatorDevice"), 12);
}

#[test]
fn each_device_arm_round_trips_in_an_op() {
    for body in [
        GovernanceEntry::DeviceKey(endorsement(1)),
        GovernanceEntry::DeviceRetired(retired(2)),
        GovernanceEntry::DeviceReadmit(readmit(3)),
        joined(Some(endorsement(4))),
        joined(None),
    ] {
        let original = op(body);
        let decoded = decode_op(&encode_op(&original).unwrap()).unwrap();
        assert_eq!(decoded.op, original);
    }
}

/// The op signature covers `MemberJoined`'s first device: its unsigned
/// form differs with and without one, and with each field changed.
#[test]
fn the_op_signature_covers_the_first_device() {
    let base = op_unsigned_bytes(&op(joined(Some(endorsement(4))))).unwrap();
    assert_ne!(op_unsigned_bytes(&op(joined(None))).unwrap(), base);
    let mut changed = endorsement(4);
    changed.incarnation += 1;
    assert_ne!(op_unsigned_bytes(&op(joined(Some(changed)))).unwrap(), base);
    let mut changed = endorsement(4);
    changed.device_record_owner.push('x');
    assert_ne!(op_unsigned_bytes(&op(joined(Some(changed)))).unwrap(), base);
}

/// The full element list fits an op and reads back; one element more
/// still fits the bytes here and the reader refuses it by count.
#[test]
fn an_endorsement_with_the_largest_extended_set_fits_one_op() {
    let mut e = endorsement(5);
    e.extends_elements = super::device_bound_tests::elements(MAX_DEVICE_SET_ELEMENTS);
    let original = op(GovernanceEntry::DeviceKey(e.clone()));
    assert_eq!(
        decode_op(&encode_op(&original).unwrap()).unwrap().op,
        original
    );
    e.extends_elements = super::device_bound_tests::elements(MAX_DEVICE_SET_ELEMENTS + 1);
    let over = op(GovernanceEntry::DeviceKey(e));
    assert!(decode_op(&encode_op(&over).unwrap()).is_err());
}

#[test]
fn a_short_device_key_is_refused() {
    use crate::governance_op_capnp::governance_op;
    // A well-formed DeviceRetired op that decodes ...
    let well_formed = encode_op(&op(GovernanceEntry::DeviceRetired(DeviceRetired {
        device_key: [1; 32],
        incarnation: 1,
        last_seq: 0,
        retirement_sig: vec![2; 64],
    })))
    .unwrap();
    assert!(decode_op(&well_formed).is_ok());
    // ... and the same wire with only the retired key one byte short.
    let read = capnp::serialize_packed::read_message(
        &mut well_formed.as_slice(),
        capnp::message::ReaderOptions::new(),
    )
    .unwrap();
    let mut builder = capnp::message::Builder::new_default();
    builder
        .set_root(read.get_root::<governance_op::Reader<'_>>().unwrap())
        .unwrap();
    let root = builder.get_root::<governance_op::Builder<'_>>().unwrap();
    let crate::community_governance_capnp::governance_entry::Which::DeviceRetired(Ok(mut d)) =
        root.get_body().unwrap().which().unwrap()
    else {
        panic!("not a DeviceRetired op");
    };
    d.set_device_key(&[1; 31]);
    let err = decode_op(&crate::capnp_codec::pack(&builder)).unwrap_err();
    assert!(
        matches!(&err, crate::CodecError::Deserialization(m) if m.starts_with("retired device key is 31 bytes")),
        "{err:?}"
    );
}

fn cert(creator_device: Option<DeviceEndorsement>) -> GenesisCert {
    GenesisCert {
        name: "c".into(),
        policy: AdmissionMode::Open,
        initial_roles: Vec::new(),
        creator_pseudonym: PseudonymKey([1; 32]),
        creator_member_record: "VLD0:m".into(),
        creator_member_record_owner: "own".into(),
        created_at: 7,
        additional_creators: Vec::new(),
        predecessor: None,
        creator_device,
    }
}

/// The creator's first endorsement rides the cert and `creatorSig`'s
/// unsigned form covers it.
#[test]
fn the_cert_carries_and_signs_the_creator_device() {
    use super::genesis::{decode_genesis, encode_genesis, SignedGenesis};
    let signed = SignedGenesis {
        cert: cert(Some(endorsement(6))),
        anchor_owner: [2; 32],
        anchor_sig: [3; 64],
        creator_sig: [4; 64],
    };
    let decoded = decode_genesis(&encode_genesis(&signed).unwrap()).unwrap();
    assert_eq!(decoded.signed.cert, signed.cert);
    let base = genesis_unsigned_bytes(&signed.cert, &[2; 32]);
    assert_eq!(decoded.unsigned, base);
    assert_ne!(genesis_unsigned_bytes(&cert(None), &[2; 32]), base);
    let mut tampered = endorsement(6);
    tampered.endorsement_sig[0] ^= 1;
    assert_ne!(
        genesis_unsigned_bytes(&cert(Some(tampered)), &[2; 32]),
        base
    );
}
