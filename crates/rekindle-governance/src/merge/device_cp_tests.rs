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
use crate::device::{
    device_set_hash, endorsement_signing_bytes, retirement_evidence_hash, retirement_signing_bytes,
};
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
    endorsement_in(a, key, &format!("VLD0:m{a}"), &format!("VLD0:d{key}"))
}

/// `who(a)`'s signed endorsement of `[key; 32]` on `device_record`, signed
/// in the context of member record `member_record` (rule 10): a re-join's
/// new `M(p)` (lead decision 63), not the record `joined(a)` names.
fn endorsement_in(a: u8, key: u8, member_record: &str, device_record: &str) -> DeviceEndorsement {
    let mut e = DeviceEndorsement {
        device_key: [key; 32],
        incarnation: u64::from(key),
        extends_set: device_set_hash(&[]),
        extends_elements: Vec::new(),
        device_record: device_record.to_owned(),
        device_record_owner: format!("VLD0:o{key}"),
        fork_evidence: [0; 32],
        endorsement_sig: Vec::new(),
    };
    let bytes = endorsement_signing_bytes(ANCHOR, member_record, &e);
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

/// `who(a)`'s signed retirement of `[key; 32]` above `last_seq`.
fn retirement(a: u8, key: u8, last_seq: u64) -> DeviceRetired {
    let mut r = DeviceRetired {
        device_key: [key; 32],
        incarnation: u64::from(key),
        last_seq,
        retirement_sig: Vec::new(),
    };
    let bytes = retirement_signing_bytes(ANCHOR, &format!("VLD0:m{a}"), &r);
    r.retirement_sig = signer(a).sign(&bytes).to_bytes().to_vec();
    r
}

/// `about`'s readmit of `[key; 32]` answering `about`'s retirement of
/// `[retired; 32]` above `last_seq` (ADR 0022 rule 7, amended 2026-11-23).
fn readmit_answering(about: u8, key: u8, retired: u8, last_seq: u64) -> GovernanceEntry {
    let record = format!("VLD0:m{about}");
    let fork_evidence =
        retirement_evidence_hash(ANCHOR, &record, &retirement(about, retired, last_seq));
    GovernanceEntry::DeviceReadmit(DeviceReadmit {
        member: who(about),
        member_record: record,
        new_key: [key; 32],
        incarnation: u64::from(key),
        fork_evidence,
    })
}

/// A device op of `who(a)` picked by `kind`: an endorsement, a retirement
/// above `last_seq`, or a readmit of `about`'s `[key; 32]` answering
/// `about`'s retirement of that key above `last_seq` (counted only where
/// such a retirement is held at its deps).
pub(super) fn device_op(a: u8, kind: usize, key: u8, last_seq: u64, about: u8) -> GovernanceEntry {
    match kind % 3 {
        0 => GovernanceEntry::DeviceKey(endorsement(a, key)),
        1 => GovernanceEntry::DeviceRetired(retirement(a, key, last_seq)),
        _ => readmit_answering(about, key, key, last_seq),
    }
}

/// The creator's community, then member 8's join with a first device and
/// each of its three kinds of device op, one op per step. The readmit is
/// member 8's own: a readmit enters the set only once applied (ADR 0022
/// rules 7 and 8, plan E3.2 K4b BUG), and member 8 has no authority over
/// another member; it is signed by key 4, endorsed before the retirement
/// (rule 7's pre-fork co-signer, K4b Q7).
fn steps() -> Vec<VerifiedOp> {
    let c0 = op(&pk(1), 0, &[], 0, joined(1, None));
    let j = op(&who(8), 0, &[&c0], 1, joined_with_device(8, 3));
    let k = op(&who(8), 1, &[&j], 2, device_op(8, 0, 4, 0, 8));
    let r = op(&who(8), 2, &[&k], 3, device_op(8, 1, 3, 1, 8));
    let m = op_on(&who(8), [4; 32], 0, &[&r], 4, readmit_answering(8, 5, 3, 1));
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
        state.devices.member(&who(8)).map(|m| m.readmits.len()),
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

/// A re-join names a new `M(p)` (lead decision 63) before the cut, and the
/// person's second device is endorsed under that new record. On top of the
/// cut, the member record current at that device's ops is the re-join's
/// (the snapshot's highest-stamp join, lead decision K4b Q4), so the resume
/// takes them exactly as the full merge does; judged by the first join's
/// record, the key would bind no current record and the ops would stall.
#[test]
fn a_rejoined_authors_second_device_resumes_on_top_of_the_cut() {
    const M2: &str = "VLD0:n8";
    let c = cert(&pk(1), AdmissionMode::Open);
    let c0 = op(&pk(1), 0, &[], 0, joined(1, None));
    let j = op(&who(8), 0, &[&c0], 1, joined_with_device(8, 3));
    let rejoin = GovernanceEntry::MemberJoined {
        member_record: M2.into(),
        member_record_owner: "VLD0:owner-2".into(),
        invite_proof: None,
        owner_proof: vec![0; 64],
        first_device: Some(endorsement_in(8, 6, M2, M2)),
    };
    let rj = op(&who(8), 1, &[&j], 2, rejoin);
    let second = GovernanceEntry::DeviceKey(endorsement_in(8, 7, M2, "VLD0:d7"));
    let k = op(&who(8), 2, &[&rj], 3, second);
    // A member without MANAGE_COMMUNITY: an op of an effective key reaches
    // validation and is refused there; an op of a key that binds no
    // current record would stall (rule 6), never reaching validation.
    let b0 = op_on(&who(8), [7; 32], 0, &[&k], 4, meta("b0"));
    let b1 = op_on(&who(8), [7; 32], 1, &[&b0], 5, meta("b1"));
    let judged = [b0.hash(), b1.hash()];
    let all = vec![c0, j, rj, k.clone(), b0, b1];
    let full = merge_outcome(&c, &all);
    assert!(
        judged.iter().all(|h| full.refused.contains(h)),
        "the second device's key is effective under the re-join's record"
    );
    let (snap, power, rest) = split(&c, &all, &[&k]);
    assert_eq!(rest.len(), 2, "the second device's ops are past the cut");
    let resumed = merge_from(&c, &snap, &power, &rest).expect("on top of the cut");
    assert!(
        judged.iter().all(|h| resumed.refused.contains(h)),
        "resume judges the key as the full merge does"
    );
    assert_eq!(resumed.state, full.state);
    assert_eq!(resumed.quarantined, full.quarantined);
}

/// The creator joins by the cert (S7d-keys.5), so its member record and
/// first device are the cert's: its own `DeviceKey` endorses a second key
/// under that record, the second key's ops apply, the merged device set
/// holds the endorsement, and the resume reaches the full merge.
#[test]
fn the_creators_second_device_is_endorsed_under_the_certs_record() {
    let creator = who(8);
    let mut c = cert(&creator, AdmissionMode::Open);
    c.creator_member_record = "VLD0:m8".into();
    c.creator_device = Some(endorsement_in(8, 3, "VLD0:m8", "VLD0:m8"));
    let c0 = op(&creator, 0, &[], 0, meta("genesis"));
    let k = op(
        &creator,
        1,
        &[&c0],
        1,
        GovernanceEntry::DeviceKey(endorsement(8, 4)),
    );
    let b0 = op_on(&creator, [4; 32], 0, &[&k], 2, meta("b0"));
    let b1 = op_on(&creator, [4; 32], 1, &[&b0], 3, meta("b1"));
    let second = [b0.hash(), b1.hash()];
    let all = vec![c0, k.clone(), b0, b1];
    let full = merge_outcome(&c, &all);
    assert!(
        second.iter().all(|h| full.applied.contains(h)),
        "the creator's second device is effective: {full:?}"
    );
    let held = full
        .state
        .devices
        .member(&creator)
        .expect("the creator's set");
    assert!(held.endorsements.contains_key(&(4, [4; 32])), "{held:?}");
    let (snap, power, rest) = split(&c, &all, &[&k]);
    assert_eq!(rest.len(), 2, "the second device's ops are past the cut");
    let resumed = merge_from(&c, &snap, &power, &rest).expect("on top of the cut");
    assert!(second.iter().all(|h| resumed.applied.contains(h)));
    assert_eq!(resumed.state, full.state);
    assert_eq!(resumed.quarantined, full.quarantined);
}
