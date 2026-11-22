//! The device set in the merged state, its checkpoint and its resume (plan
//! E3.2 K-CP, ADR 0022 rules 2, 5–7): the state root covers it, a snapshot
//! round-trips it, and `merge_from` reaches the full merge's set
//! (`resume_tests`' generator carries device ops through these helpers).

use rekindle_secrets::ed25519_dalek::{Signer, SigningKey};
use rekindle_types::governance::{
    AdmissionMode, Checkpoint, DeviceEndorsement, DeviceReadmit, DeviceRetired, GovernanceEntry,
};
use rekindle_types::id::PseudonymKey;

use super::op_proptests::{joined, pk};
use super::resume_tests::split;
use super::{merge, merge_cut, merge_from, merge_outcome};
use crate::checkpoint::{state_preimage, state_root_of, CutSnapshot};
use crate::checksum::canonical_state;
use crate::device::{device_set_hash, endorsement_signing_bytes, retirement_signing_bytes};
use crate::fixture::{cert, meta, op, op_on, FIXTURE_ANCHOR as ANCHOR};
use crate::op::VerifiedOp;

fn signer(a: u8) -> SigningKey {
    SigningKey::from_bytes(&[a; 32])
}

/// A generator author: 8 and up hold a real key and sign device elements;
/// below 8, `op_proptests`' fixture pseudonyms.
pub(super) fn who(a: u8) -> PseudonymKey {
    if a >= 8 {
        PseudonymKey(signer(a).verifying_key().to_bytes())
    } else {
        pk(a)
    }
}

/// `who(a)`'s signed endorsement of `[key; 32]`, in the record `joined(a)`
/// names.
fn endorsement(a: u8, key: u8) -> DeviceEndorsement {
    let mut e = DeviceEndorsement {
        device_key: [key; 32],
        incarnation: u64::from(key),
        extends_set: device_set_hash(&[]),
        extends_elements: Vec::new(),
        device_record: format!("VLD0:d{key}"),
        device_record_owner: format!("VLD0:o{key}"),
        fork_evidence: [0; 32],
        endorsement_sig: Vec::new(),
    };
    let bytes = endorsement_signing_bytes(ANCHOR, &format!("VLD0:m{a}"), &e);
    e.endorsement_sig = signer(a).sign(&bytes).to_bytes().to_vec();
    e
}

/// `who(a)`'s join carrying its first device `[key; 32]`.
pub(super) fn joined_with_device(a: u8, key: u8) -> GovernanceEntry {
    let mut entry = joined(a, Some("code"));
    if let GovernanceEntry::MemberJoined { first_device, .. } = &mut entry {
        *first_device = Some(endorsement(a, key));
    }
    entry
}

/// A device op of `who(a)` picked by `kind`: an endorsement, a retirement
/// above `last_seq`, or a readmit of `about`'s `[key; 32]`.
pub(super) fn device_op(a: u8, kind: usize, key: u8, last_seq: u64, about: u8) -> GovernanceEntry {
    match kind % 3 {
        0 => GovernanceEntry::DeviceKey(endorsement(a, key)),
        1 => {
            let mut r = DeviceRetired {
                device_key: [key; 32],
                incarnation: u64::from(key),
                last_seq,
                retirement_sig: Vec::new(),
            };
            let bytes = retirement_signing_bytes(ANCHOR, &format!("VLD0:m{a}"), &r);
            r.retirement_sig = signer(a).sign(&bytes).to_bytes().to_vec();
            GovernanceEntry::DeviceRetired(r)
        }
        _ => GovernanceEntry::DeviceReadmit(DeviceReadmit {
            member: who(about),
            member_record: format!("VLD0:m{about}"),
            new_key: [key; 32],
            incarnation: u64::from(key),
            fork_evidence: [1; 32],
        }),
    }
}

/// The creator's community, then member 8's join with a first device and
/// each of its three kinds of device op, one op per step.
fn steps() -> Vec<VerifiedOp> {
    let c0 = op(&pk(1), 0, &[], 0, joined(1, None));
    let j = op(&who(8), 0, &[&c0], 1, joined_with_device(8, 3));
    let k = op(&who(8), 1, &[&j], 2, device_op(8, 0, 4, 0, 8));
    let r = op(&who(8), 2, &[&k], 3, device_op(8, 1, 3, 1, 8));
    let m = op(&who(8), 3, &[&r], 4, device_op(8, 2, 5, 0, 9));
    vec![c0, j, k, r, m]
}

/// Each device element merged changes the state root: the first device a
/// join carries, a further endorsement, a retirement and a readmit.
#[test]
fn the_state_root_changes_when_a_member_s_device_set_changes() {
    let c = cert(&pk(1), AdmissionMode::Open);
    let all = steps();
    let roots: Vec<Vec<u8>> = (1..=all.len())
        .map(|n| canonical_state(&merge(&c, &all[..n])))
        .collect();
    for pair in roots.windows(2) {
        assert_ne!(pair[0], pair[1], "a device element left the root unchanged");
    }
    let state = merge(&c, &all);
    let md = state.devices.member(&who(8)).expect("member 8's set");
    assert_eq!(md.endorsements.len(), 2);
    assert_eq!(md.retirements.len(), 1);
    assert_eq!(
        state.devices.member(&who(9)).map(|m| m.readmits.len()),
        Some(1)
    );
}

/// A checkpoint snapshot decodes back to the device set it was cut with.
#[test]
fn a_checkpoint_round_trip_keeps_the_device_set() {
    let c = cert(&pk(1), AdmissionMode::Open);
    let (state, cut) = merge_cut(&c, &steps());
    assert!(state.devices.member(&who(8)).is_some());
    let bytes = state_preimage(&state, &cut);
    let checkpoint = Checkpoint {
        height: 1,
        round: 0,
        prev: None,
        cut: Vec::new(),
        frontier_hash: [0; 32],
        dir_root: [0; 32],
        dir_size: 0,
        state_root: state_root_of(&bytes),
        state_size: u32::try_from(bytes.len()).expect("a test snapshot fits"),
        mek_generation: 1,
        eligible_set_hash: [0; 32],
        pages: Vec::new(),
    };
    let snap = CutSnapshot::decode(&checkpoint, &bytes).expect("a canonical snapshot");
    assert_eq!(snap.state.devices, state.devices);
}

/// A seq numbers its device key's chain (ADR 0022 rule 1), so the cut is
/// per device key: member 8's second device writes seq 0 and 1 of its own
/// chain after a cut that holds its first device's seq 0 and 1. Keyed by
/// author, the first device's top would cover them (`IntoTheCut`), and the
/// full merge would call the two seq 0 ops a fork, muting the person
/// (rule 5: a fork retires the key, not the person).
#[test]
fn a_second_device_s_ops_past_its_own_cut_resume_as_the_full_merge() {
    let c = cert(&pk(1), AdmissionMode::Open);
    let c0 = op(&pk(1), 0, &[], 0, joined(1, None));
    let j = op(&who(8), 0, &[&c0], 1, joined_with_device(8, 3));
    let k = op(&who(8), 1, &[&j], 2, device_op(8, 0, 4, 0, 8));
    let b0 = op_on(&who(8), [4; 32], 0, &[&k], 3, meta("b0"));
    let b1 = op_on(&who(8), [4; 32], 1, &[&b0], 4, meta("b1"));
    let all = vec![c0, j, k.clone(), b0, b1];
    let full = merge_outcome(&c, &all);
    assert!(
        full.equivocators.is_empty(),
        "two devices' seq 0 ops are no fork"
    );
    let (snap, power, rest) = split(&c, &all, &[&k]);
    assert_eq!(rest.len(), 2, "the second device's ops are past the cut");
    let resumed = merge_from(&c, &snap, &power, &rest).expect("on top of the cut");
    assert_eq!(resumed.state, full.state);
    assert_eq!(resumed.quarantined, full.quarantined);
}
