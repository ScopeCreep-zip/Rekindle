//! The per-op device-key judge in `prepare` (plan E3.2 S7d-keys K4b.1,
//! ADR 0022 rules 1 and 6, lead finding F1).

use rekindle_secrets::ed25519_dalek::{Signer, SigningKey};
use rekindle_types::governance::{AdmissionMode, DeviceEndorsement, GovernanceEntry};
use rekindle_types::id::PseudonymKey;

use crate::device::{device_set_hash, endorsement_signing_bytes};
use crate::device_judge::KeyRefusal;
use crate::fixture::{cert, meta, op, op_on, FIXTURE_ANCHOR};
use crate::merge::device_cp_tests::{device_op, joined_with_device, who};
use crate::merge::op_proptests::{joined, pk};
use crate::merge::resume_tests::split;
use crate::merge::{merge_from, merge_outcome, MergeOutcome, NotOnTop, QuarantineReason};
use crate::op::VerifiedOp;

/// A member whose pseudonym is `seed`'s real key (its endorsements verify).
fn member(seed: u8) -> PseudonymKey {
    PseudonymKey(
        SigningKey::from_bytes(&[seed; 32])
            .verifying_key()
            .to_bytes(),
    )
}

/// `member(seed)`'s endorsement of `[key; 32]` at `incarnation`, signed
/// for its member record `record`.
fn endorse(seed: u8, key: u8, incarnation: u64, record: &str) -> DeviceEndorsement {
    let mut e = DeviceEndorsement {
        device_key: [key; 32],
        incarnation,
        extends_set: device_set_hash(&[]),
        extends_elements: Vec::new(),
        device_record: format!("VLD0:device-{key}"),
        device_record_owner: format!("VLD0:owner-{key}"),
        fork_evidence: [0; 32],
        endorsement_sig: Vec::new(),
    };
    let bytes = endorsement_signing_bytes(FIXTURE_ANCHOR, record, &e);
    e.endorsement_sig = (SigningKey::from_bytes(&[seed; 32]).sign(&bytes).to_bytes()).to_vec();
    e
}

fn join(record: &str, first: Option<DeviceEndorsement>) -> GovernanceEntry {
    GovernanceEntry::MemberJoined {
        member_record: record.into(),
        member_record_owner: "owner".into(),
        invite_proof: None,
        owner_proof: vec![0; 64],
        first_device: first,
    }
}

fn outcome(ops: &[&VerifiedOp]) -> MergeOutcome {
    let ops: Vec<VerifiedOp> = ops.iter().map(|o| (*o).clone()).collect();
    merge_outcome(&cert(&pk(1), AdmissionMode::Open), &ops)
}

fn quarantined(out: &MergeOutcome, op: &VerifiedOp) -> bool {
    out.quarantined.iter().any(|(h, _)| *h == op.hash())
}

/// A key its author never endorsed: its op is pending (a missing dep,
/// ADR 0022 rule 6), never quarantined, and so is every op citing it; the
/// author's endorsed key beside it is not.
#[test]
fn an_unendorsed_key_s_op_and_its_dependants_are_pending() {
    let m = member(2);
    let j = op(
        &m,
        0,
        &[],
        0,
        join("VLD0:r", Some(endorse(2, 10, 1, "VLD0:r"))),
    );
    let good = op_on(&m, [10; 32], 0, &[&j], 1, meta("good"));
    let bad = op_on(&m, [11; 32], 0, &[&j], 2, meta("bad"));
    let after = op(&m, 1, &[&j, &bad], 3, meta("after"));
    let out = outcome(&[&j, &good, &bad, &after]);
    assert!(!out.pending.contains(&good.hash()) && !quarantined(&out, &good));
    assert!(out.pending.contains(&bad.hash()) && !quarantined(&out, &bad));
    assert!(out.pending.contains(&after.hash()));
}

/// A key endorsed only under a member record its author has since left is
/// pending for the re-join's record (ADR 0022 rule 6, lead decision K4b
/// Q3), and effective at the first join's deps.
#[test]
fn a_stale_key_after_rejoin_is_not_effective() {
    let m = member(2);
    let j1 = op(
        &m,
        0,
        &[],
        0,
        join("VLD0:r1", Some(endorse(2, 10, 1, "VLD0:r1"))),
    );
    let fresh = op_on(&m, [10; 32], 0, &[&j1], 1, meta("fresh"));
    let out = outcome(&[&j1, &fresh]);
    assert!(!out.pending.contains(&fresh.hash()) && !quarantined(&out, &fresh));
    let left = op(
        &m,
        1,
        &[&j1],
        2,
        GovernanceEntry::MemberLeft { heads: Vec::new() },
    );
    let j2 = op(&m, 2, &[&left], 3, join("VLD0:r2", None));
    let stale = op_on(&m, [10; 32], 0, &[&j2], 4, meta("stale"));
    let out = outcome(&[&j1, &left, &j2, &stale]);
    assert!(out.pending.contains(&stale.hash()) && !quarantined(&out, &stale));
}

/// A seventh live key is refused (ADR 0022 rule 6): its op is quarantined
/// with the refusal, never applied nor pending.
#[test]
fn a_seventh_key_s_op_is_quarantined_with_its_reason() {
    let m = member(2);
    let mut ops = vec![op(
        &m,
        0,
        &[],
        0,
        join("VLD0:r", Some(endorse(2, 10, 1, "VLD0:r"))),
    )];
    for key in 11..=16u8 {
        let e = endorse(2, key, u64::from(key), "VLD0:r");
        let prev = ops.last().expect("the join").clone();
        let seq = u64::from(key - 10);
        ops.push(op(&m, seq, &[&prev], seq, GovernanceEntry::DeviceKey(e)));
    }
    let last = ops.last().expect("the sixth endorsement").clone();
    let seventh = op_on(&m, [16; 32], 0, &[&last], 20, meta("seventh"));
    ops.push(seventh.clone());
    let out = outcome(&ops.iter().collect::<Vec<_>>());
    let reason = QuarantineReason::DeviceKey {
        refusal: KeyRefusal::OverCap,
    };
    assert!((out.quarantined.iter()).any(|(h, r)| *h == seventh.hash() && *r == reason));
    assert!(!out.applied.contains(&seventh.hash()) && !out.pending.contains(&seventh.hash()));
}

/// `merge_from` never continues past an op whose key is not effective for
/// its author: a forged author on top of the cut is `DeviceJudged`.
#[test]
fn a_forged_author_on_top_of_the_cut_is_device_judged() {
    let c = cert(&pk(1), AdmissionMode::Open);
    let c0 = op(&pk(1), 0, &[], 0, joined(1, None));
    let forged = op_on(&pk(1), [77; 32], 0, &[&c0], 1, meta("forged"));
    let all = [c0.clone(), forged.clone()];
    let (snap, power, rest) = split(&c, &all, &[&c0]);
    assert_eq!(
        merge_from(&c, &snap, &power, &rest).err(),
        Some(NotOnTop::DeviceJudged(forged.hash()))
    );
}

/// An op beside the cut whose past reaches its author's latest join is
/// decided from the snapshot (`RestJudge`): the resume stands and equals
/// the full merge (netsim `racing_join`: the hint is kept).
#[test]
fn an_op_beside_the_cut_that_sees_its_authors_join_resumes() {
    let c = cert(&pk(1), AdmissionMode::Open);
    let c0 = op(&pk(1), 0, &[], 0, joined(1, None));
    let j = op(&who(8), 0, &[&c0], 1, joined_with_device(8, 3));
    // The creator's op the member's next op never sees: a cut head.
    let x = op(&pk(1), 1, &[&c0], 2, meta("x"));
    // Member 8 holds no MANAGE_COMMUNITY: an effective key's op reaches the
    // permission check and is refused; an ineffective one would stall.
    let b = op_on(&who(8), [3; 32], 0, &[&j], 3, meta("b"));
    let all = [c0.clone(), j.clone(), x.clone(), b.clone()];
    let full = merge_outcome(&c, &all);
    assert!(full.refused.contains(&b.hash()), "the key is effective");
    let (snap, power, rest) = split(&c, &all, &[&j, &x]);
    assert_eq!(rest.len(), 1, "only the member's op is past the cut");
    let resumed = merge_from(&c, &snap, &power, &rest).expect("beside the cut, judged");
    assert!(resumed.refused.contains(&b.hash()));
    assert_eq!(resumed.state, full.state);
}

/// An endorsement carried by an op in the cut that an op beside the cut
/// never saw (rule 2: any op may carry one) is not in that op's past: the
/// full merge holds its key unendorsed (pending), so the resume must not
/// judge it effective from the snapshot; it answers `DeviceJudged`.
#[test]
fn an_endorsement_carried_where_an_op_beside_the_cut_never_looked_is_not_its() {
    let c = cert(&pk(1), AdmissionMode::Open);
    let c0 = op(&pk(1), 0, &[], 0, joined(1, None));
    let j = op(&who(8), 0, &[&c0], 1, joined_with_device(8, 3));
    // The pseudonym carries key 4's endorsement in an op of its own.
    let k = op(&who(8), 1, &[&j], 2, device_op(8, 0, 4, 0, 8));
    // Key 4's first op cites only the join, never the endorsement.
    let b = op_on(&who(8), [4; 32], 0, &[&j], 3, meta("b"));
    let all = [c0.clone(), j.clone(), k.clone(), b.clone()];
    let full = merge_outcome(&c, &all);
    assert!(full.pending.contains(&b.hash()), "unendorsed at its deps");
    let (snap, power, rest) = split(&c, &all, &[&k]);
    assert_eq!(rest.len(), 1, "only key 4's op is past the cut");
    assert!(matches!(
        merge_from(&c, &snap, &power, &rest),
        Err(NotOnTop::DeviceJudged(h)) if h == b.hash()
    ));
}

/// An op beside the cut that does not reach its author's latest join (a
/// re-join in the cut it never saw) is not judged from the snapshot: the
/// record current at its deps may be the earlier join's, so the resume
/// answers `DeviceJudged` and the full merge judges it.
#[test]
fn an_op_beside_the_cut_that_misses_its_authors_rejoin_is_device_judged() {
    let c = cert(&pk(1), AdmissionMode::Open);
    let c0 = op(&pk(1), 0, &[], 0, joined(1, None));
    let m = member(2);
    let j1 = op(
        &m,
        0,
        &[&c0],
        1,
        join("VLD0:r1", Some(endorse(2, 10, 1, "VLD0:r1"))),
    );
    let left = op(
        &m,
        1,
        &[&j1],
        2,
        GovernanceEntry::MemberLeft { heads: Vec::new() },
    );
    let j2 = op(&m, 2, &[&left], 3, join("VLD0:r2", None));
    let b = op_on(&m, [10; 32], 0, &[&j1], 4, meta("b"));
    let all = [c0.clone(), j1.clone(), left.clone(), j2.clone(), b.clone()];
    let (snap, power, rest) = split(&c, &all, &[&j2]);
    assert_eq!(rest.len(), 1, "only the op beside the cut is past it");
    assert!(matches!(
        merge_from(&c, &snap, &power, &rest),
        Err(NotOnTop::DeviceJudged(h)) if h == b.hash()
    ));
}

/// F1 regression: a key no one endorsed signs an op naming member 1 as
/// its author; the op is not applied (pending: its key is unendorsed for
/// that author at its deps), and nor is the op that cites it.
#[test]
fn a_forged_author_is_not_applied() {
    let c0 = op(&pk(1), 0, &[], 0, joined(1, None));
    let forged = op_on(&pk(1), [77; 32], 0, &[&c0], 1, meta("forged"));
    let after = op(&pk(1), 1, &[&c0, &forged], 2, meta("after"));
    let set = [c0.clone(), forged.clone(), after.clone()];
    let out = merge_outcome(&cert(&pk(1), AdmissionMode::Open), &set);
    assert!(!out.applied.contains(&forged.hash()));
    assert!(out.pending.contains(&forged.hash()));
    assert!(out.pending.contains(&after.hash()));
    assert!(out.applied.contains(&c0.hash()));
}

/// The interim binding (lead decision K4b Q1): an op signed by its
/// author's pseudonym is author-bound by its signature (removed at
/// S7d-keys.5).
#[test]
fn a_pseudonym_signed_op_is_effective() {
    let c0 = op(&pk(1), 0, &[], 0, joined(1, None));
    let renamed = op(&pk(1), 1, &[&c0], 1, meta("renamed"));
    let set = [c0.clone(), renamed.clone()];
    let out = merge_outcome(&cert(&pk(1), AdmissionMode::Open), &set);
    assert!(out.applied.contains(&renamed.hash()));
    assert!(out.pending.is_empty());
}
