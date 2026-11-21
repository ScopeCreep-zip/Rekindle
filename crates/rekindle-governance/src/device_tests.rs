//! Rule 10's layouts: each signed field is covered (tampering any one
//! breaks the signature), the context binds, and the set hash is canonical.

use rekindle_secrets::ed25519_dalek::{Signer, SigningKey};
use rekindle_types::governance::{
    DeviceEndorsement, DeviceReadmit, DeviceRetired, DeviceSetElement, DeviceSetElementKind, OpHash,
};
use rekindle_types::id::PseudonymKey;

use super::{
    device_set_hash, endorsement_holds, endorsement_signing_bytes, endorsement_well_formed,
    fork_evidence_hash, readmit_body_bytes, retirement_holds, retirement_signing_bytes,
};

const COMMUNITY: &str = "VLD0:community";
const RECORD: &str = "VLD0:member";

fn key() -> SigningKey {
    SigningKey::from_bytes(&[7; 32])
}

fn pk() -> PseudonymKey {
    PseudonymKey(key().verifying_key().to_bytes())
}

fn element(kind: DeviceSetElementKind, b: u8, incarnation: u64) -> DeviceSetElement {
    DeviceSetElement {
        kind,
        device_key: [b; 32],
        incarnation,
    }
}

/// A signed endorsement of `[9; 32]` extending one endorsed, retired key.
fn endorsement() -> DeviceEndorsement {
    let elements = vec![
        element(DeviceSetElementKind::Endorsement, 1, 1),
        element(DeviceSetElementKind::Retirement, 1, 1),
    ];
    let mut e = DeviceEndorsement {
        device_key: [9; 32],
        incarnation: 2,
        extends_set: device_set_hash(&elements),
        extends_elements: elements,
        device_record: "VLD0:device".into(),
        device_record_owner: "VLD0:device-owner".into(),
        fork_evidence: [0; 32],
        endorsement_sig: Vec::new(),
    };
    e.endorsement_sig = key()
        .sign(&endorsement_signing_bytes(COMMUNITY, RECORD, &e))
        .to_bytes()
        .to_vec();
    e
}

#[test]
fn a_signed_endorsement_holds_in_its_own_context_only() {
    let e = endorsement();
    assert!(endorsement_holds(&pk(), COMMUNITY, RECORD, &e));
    assert!(!endorsement_holds(&pk(), "VLD0:other", RECORD, &e));
    assert!(!endorsement_holds(&pk(), COMMUNITY, "VLD0:other", &e));
    assert!(!endorsement_holds(
        &PseudonymKey([3; 32]),
        COMMUNITY,
        RECORD,
        &e
    ));
}

/// Every rule-10 field is under the signature: change any one and it no
/// longer verifies.
#[test]
fn the_endorsement_signature_covers_every_rule_ten_field() {
    type Tamper = fn(&mut DeviceEndorsement);
    let tampers: [(&str, Tamper); 6] = [
        ("deviceKey", |e| e.device_key[0] ^= 1),
        ("incarnation", |e| e.incarnation += 1),
        ("extendsSet", |e| e.extends_set[0] ^= 1),
        ("deviceRecord", |e| e.device_record.push('x')),
        ("deviceRecordOwner", |e| e.device_record_owner.push('x')),
        ("forkEvidence", |e| e.fork_evidence[0] ^= 1),
    ];
    for (field, tamper) in tampers {
        let mut e = endorsement();
        let base = endorsement_signing_bytes(COMMUNITY, RECORD, &e);
        tamper(&mut e);
        assert_ne!(
            endorsement_signing_bytes(COMMUNITY, RECORD, &e),
            base,
            "{field}"
        );
        let sig = <[u8; 64]>::try_from(e.endorsement_sig.as_slice()).unwrap();
        assert!(
            rekindle_secrets::sign::verify_signature(
                &pk().0,
                &endorsement_signing_bytes(COMMUNITY, RECORD, &e),
                &sig
            )
            .is_err(),
            "{field} is not under the signature"
        );
    }
}

/// The element list travels with the hash: a list that does not hash to
/// `extendsSet` is malformed, and so is one an incarnation does not exceed.
#[test]
fn an_extended_set_must_hash_and_sit_below_the_incarnation() {
    let e = endorsement();
    assert!(endorsement_well_formed(&e));
    let mut other = e.clone();
    other.extends_elements.pop();
    assert!(!endorsement_well_formed(&other));
    let mut low = e.clone();
    low.incarnation = 1;
    assert!(
        !endorsement_well_formed(&low),
        "incarnation not above the set"
    );
    let mut unsorted = e;
    unsorted.extends_elements.reverse();
    unsorted.extends_set = device_set_hash(&unsorted.extends_elements);
    assert!(!endorsement_well_formed(&unsorted), "one canonical order");
}

#[test]
fn the_set_hash_is_order_free_and_kind_bound() {
    let a = element(DeviceSetElementKind::Endorsement, 1, 1);
    let b = element(DeviceSetElementKind::Endorsement, 2, 1);
    assert_eq!(device_set_hash(&[a, b]), device_set_hash(&[b, a]));
    let r = element(DeviceSetElementKind::Retirement, 1, 1);
    assert_ne!(device_set_hash(&[a]), device_set_hash(&[r]));
    assert_ne!(device_set_hash(&[]), device_set_hash(&[a]));
}

#[test]
fn the_retirement_signature_covers_every_rule_ten_field() {
    let mut r = DeviceRetired {
        device_key: [9; 32],
        incarnation: 2,
        last_seq: 40,
        retirement_sig: Vec::new(),
    };
    r.retirement_sig = key()
        .sign(&retirement_signing_bytes(COMMUNITY, RECORD, &r))
        .to_bytes()
        .to_vec();
    assert!(retirement_holds(&pk(), COMMUNITY, RECORD, &r));
    assert!(!retirement_holds(&pk(), "VLD0:other", RECORD, &r));
    assert!(!retirement_holds(&pk(), COMMUNITY, "VLD0:other", &r));
    type Tamper = fn(&mut DeviceRetired);
    let tampers: [Tamper; 3] = [
        |r| r.device_key[0] ^= 1,
        |r| r.incarnation += 1,
        |r| r.last_seq += 1,
    ];
    for tamper in tampers {
        let mut t = r.clone();
        tamper(&mut t);
        assert!(!retirement_holds(&pk(), COMMUNITY, RECORD, &t));
    }
}

#[test]
fn the_readmit_body_binds_every_rule_ten_field() {
    let r = DeviceReadmit {
        member: PseudonymKey([1; 32]),
        member_record: RECORD.into(),
        new_key: [2; 32],
        incarnation: 3,
        fork_evidence: [4; 32],
    };
    let base = readmit_body_bytes(COMMUNITY, &r);
    assert_ne!(readmit_body_bytes("VLD0:other", &r), base);
    type Tamper = fn(&mut DeviceReadmit);
    let tampers: [Tamper; 4] = [
        |r| r.member_record.push('x'),
        |r| r.new_key[0] ^= 1,
        |r| r.incarnation += 1,
        |r| r.fork_evidence[0] ^= 1,
    ];
    for tamper in tampers {
        let mut t = r.clone();
        tamper(&mut t);
        assert_ne!(readmit_body_bytes(COMMUNITY, &t), base);
    }
}

#[test]
fn fork_evidence_is_order_free_and_bound_to_key_and_seq() {
    let (a, b) = (OpHash([1; 32]), OpHash([2; 32]));
    let h = fork_evidence_hash(COMMUNITY, &[9; 32], 4, [a, b]);
    assert_eq!(fork_evidence_hash(COMMUNITY, &[9; 32], 4, [b, a]), h);
    assert_ne!(fork_evidence_hash(COMMUNITY, &[9; 32], 5, [a, b]), h);
    assert_ne!(fork_evidence_hash(COMMUNITY, &[8; 32], 4, [a, b]), h);
    assert_ne!(fork_evidence_hash("VLD0:other", &[9; 32], 4, [a, b]), h);
}
