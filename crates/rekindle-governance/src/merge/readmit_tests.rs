//! `DeviceReadmit` authority (ADR 0022 rules 7 and 8, plan E3.2 K4b BUG
//! and Q6): a readmit enters the device view only once applied, and a
//! moderator's is judged at its causal position, as a kick is.

use rekindle_secrets::ed25519_dalek::{Signer, SigningKey};
use rekindle_types::governance::{
    AdmissionMode, DeviceEndorsement, DeviceReadmit, DeviceRetired, GovernanceEntry,
};
use rekindle_types::id::{PseudonymKey, RoleId};

use super::judge::tests::outcome;
use super::op_proptests::{joined, pk, role};
use super::resume::NotOnTop;
use super::resume_tests::resume;
use super::{MergeOutcome, QuarantineReason};
use crate::device::{
    device_set_hash, endorsement_signing_bytes, fork_evidence_hash, retirement_evidence_hash,
    retirement_signing_bytes,
};
use crate::device_judge::KeyRefusal;
use crate::fixture::{cert, meta, op, op_on, FIXTURE_ANCHOR};
use crate::op::VerifiedOp;

const RECORD: &str = "VLD0:r";

/// The forked member: its pseudonym is seed 2's real key.
fn t() -> PseudonymKey {
    PseudonymKey(SigningKey::from_bytes(&[2; 32]).verifying_key().to_bytes())
}

/// `t()`'s endorsement of `[key; 32]` at `incarnation`, answering `fork`.
fn endorse(key: u8, incarnation: u64, fork: [u8; 32]) -> DeviceEndorsement {
    let mut e = DeviceEndorsement {
        device_key: [key; 32],
        incarnation,
        extends_set: device_set_hash(&[]),
        extends_elements: Vec::new(),
        device_record: format!("VLD0:device-{key}"),
        device_record_owner: format!("VLD0:owner-{key}"),
        fork_evidence: fork,
        endorsement_sig: Vec::new(),
    };
    let bytes = endorsement_signing_bytes(FIXTURE_ANCHOR, RECORD, &e);
    e.endorsement_sig = (SigningKey::from_bytes(&[2; 32]).sign(&bytes).to_bytes()).to_vec();
    e
}

/// `t()`'s readmit of `[key; 32]` at `incarnation`, answering `fork`.
fn readmit_of(key: u8, incarnation: u64, fork: [u8; 32]) -> GovernanceEntry {
    GovernanceEntry::DeviceReadmit(DeviceReadmit {
        member: t(),
        member_record: RECORD.into(),
        new_key: [key; 32],
        incarnation,
        fork_evidence: fork,
    })
}

/// `t()`'s readmit of key 20 (incarnation 2), answering `fork`.
fn readmit(fork: [u8; 32]) -> GovernanceEntry {
    readmit_of(20, 2, fork)
}

/// `t()`'s retirement of key 10 (incarnation 1) above seq 0.
fn retired_10() -> DeviceRetired {
    let mut r = DeviceRetired {
        device_key: [10; 32],
        incarnation: 1,
        last_seq: 0,
        retirement_sig: Vec::new(),
    };
    let bytes = retirement_signing_bytes(FIXTURE_ANCHOR, RECORD, &r);
    r.retirement_sig = (SigningKey::from_bytes(&[2; 32]).sign(&bytes).to_bytes()).to_vec();
    r
}

fn retire_10() -> GovernanceEntry {
    GovernanceEntry::DeviceRetired(retired_10())
}

/// The evidence of `retire_10` (ADR 0022 rule 10, amended 2026-11-23).
fn retirement_10() -> [u8; 32] {
    retirement_evidence_hash(FIXTURE_ANCHOR, RECORD, &retired_10())
}

/// `t()`'s join with key 10.
fn join_t() -> VerifiedOp {
    let join = GovernanceEntry::MemberJoined {
        member_record: RECORD.into(),
        member_record_owner: "owner".into(),
        invite_proof: None,
        owner_proof: vec![0; 64],
        first_device: Some(endorse(10, 1, [0; 32])),
    };
    op(&t(), 0, &[], 0, join)
}

/// `t()`'s endorsement of key 20 answering `fork`, its op at `seq`.
fn endorse_20(seq: u64, deps: &[&VerifiedOp], at: u64, fork: [u8; 32]) -> VerifiedOp {
    let e = GovernanceEntry::DeviceKey(endorse(20, 2, fork));
    op(&t(), seq, deps, at, e)
}

fn grant(target: PseudonymKey, id: u8) -> GovernanceEntry {
    GovernanceEntry::RoleAssignment {
        target,
        role_id: RoleId([id; 16]),
    }
}

/// `t()` joined with key 10, its retirement of key 10 and its
/// endorsement of key 20 answering that retirement: key 20 needs a
/// readmit. With `retired_first` the retirement precedes the endorsement,
/// else it follows it. Returns the join, the endorsement and the
/// retirement.
fn forked_member(retired_first: bool) -> (VerifiedOp, VerifiedOp, VerifiedOp) {
    let tj = join_t();
    if retired_first {
        let ret = op(&t(), 1, &[&tj], 1, retire_10());
        let dk = endorse_20(2, &[&ret], 2, retirement_10());
        (tj, dk, ret)
    } else {
        let dk = endorse_20(1, &[&tj], 1, retirement_10());
        let ret = op(&t(), 2, &[&dk], 2, retire_10());
        (tj, dk, ret)
    }
}

fn needs_readmit(out: &MergeOutcome, o: &VerifiedOp) -> bool {
    let reason = QuarantineReason::DeviceKey {
        refusal: KeyRefusal::NeedsReadmit,
    };
    (out.quarantined.iter()).any(|(h, r)| *h == o.hash() && *r == reason)
}

/// A readmit its author has no authority for (no `KICK_MEMBERS`) is
/// refused and never enters the device view: the key it names still needs
/// a readmit (K4b BUG).
#[test]
fn an_unauthorised_readmit_never_makes_its_key_effective() {
    let (tj, dk, ret) = forked_member(true);
    let x = op(&pk(5), 0, &[], 2, joined(5, None));
    let r = op(&pk(5), 1, &[&x, &dk], 3, readmit(retirement_10()));
    let after = op_on(&t(), [20; 32], 0, &[&r], 4, meta("after"));
    let out = outcome(&[&tj, &ret, &dk, &x, &r, &after]);
    assert!(out.refused.contains(&r.hash()), "the readmit is refused");
    assert!(needs_readmit(&out, &after), "{out:?}");
    assert!(!out.applied.contains(&after.hash()));
}

/// Creator 1 defines role 9 (`KICK_MEMBERS`, rank 50) and grants it to
/// member 3, and gives `t()` role 8 at `t_rank`; member 3's readmit of
/// `t()`'s key 20 cites the grant when `cites_grant`, and its deps hold
/// the retirement it answers when `retired_first`. Returns the outcome
/// and the op of key 20 after the readmit.
fn moderated(cites_grant: bool, t_rank: u32, retired_first: bool) -> (MergeOutcome, VerifiedOp) {
    let (tj, dk, ret) = forked_member(retired_first);
    let r9 = op(&pk(1), 0, &[], 2, role(9, 50));
    let r8 = op(&pk(1), 1, &[&r9], 3, role(8, t_rank));
    let mj = op(&pk(3), 0, &[], 4, joined(3, None));
    let g = op(&pk(1), 2, &[&r8, &mj], 5, grant(pk(3), 9));
    let tg = op(&pk(1), 3, &[&g, &dk], 6, grant(t(), 8));
    let deps: Vec<&VerifiedOp> = if cites_grant {
        vec![&mj, &dk, &tg]
    } else {
        vec![&mj, &dk]
    };
    let r = op(&pk(3), 1, &deps, 7, readmit(retirement_10()));
    let after = op_on(&t(), [20; 32], 0, &[&r], 8, meta("after"));
    (
        outcome(&[&tj, &ret, &dk, &r9, &r8, &mj, &g, &tg, &r, &after]),
        after,
    )
}

/// A moderator's readmit is judged against the power state in its causal
/// past (ADR 0022 rule 8, plan E3.2 K4b Q6), not at genesis: with the
/// grant in its deps it applies and the key is effective; with the grant
/// concurrent, or the member at the moderator's rank, it is refused.
#[test]
fn a_moderator_readmit_is_judged_at_its_causal_rank() {
    let (out, after) = moderated(true, 10, true);
    assert!(out.applied.contains(&after.hash()), "{out:?}");
    let (out, after) = moderated(false, 10, true);
    assert!(needs_readmit(&out, &after), "grant concurrent: {out:?}");
    let (out, after) = moderated(true, 50, true);
    assert!(needs_readmit(&out, &after), "not outranked: {out:?}");
}

/// `t()`'s own readmit of key 20, signed by key 11, whose endorsement both
/// ops of key 10's fork cite when `both`, else only the first; key 20's
/// endorsement and the readmit name the pair's hash. Returns the outcome,
/// the readmit and the op of key 20 after it.
fn own_readmit(both: bool) -> (MergeOutcome, VerifiedOp, VerifiedOp) {
    let tj = join_t();
    let e11 = op(
        &t(),
        1,
        &[&tj],
        1,
        GovernanceEntry::DeviceKey(endorse(11, 3, [0; 32])),
    );
    let f1 = op_on(&t(), [10; 32], 0, &[&e11], 2, meta("a"));
    let f2_deps: [&VerifiedOp; 1] = if both { [&e11] } else { [&tj] };
    let f2 = op_on(&t(), [10; 32], 0, &f2_deps, 3, meta("b"));
    let pair = fork_evidence_hash(FIXTURE_ANCHOR, &[10; 32], 0, [f1.hash(), f2.hash()]);
    let dk = endorse_20(2, &[&e11, &f1, &f2], 4, pair);
    let r = op_on(&t(), [11; 32], 0, &[&dk], 5, readmit(pair));
    // A leave: `t()` holds no role, and a leave needs none.
    let leave = GovernanceEntry::MemberLeft { heads: Vec::new() };
    let after = op_on(&t(), [20; 32], 0, &[&r], 6, leave);
    let out = outcome(&[&tj, &dk, &e11, &f1, &f2, &r, &after]);
    (out, r, after)
}

/// An own-key readmit's co-signer is pre-fork only when its endorsement
/// is in the strict causal past of every fork event its deps hold, both
/// ops of a fork pair (ADR 0022 rule 7, plan E3.2 K4b Q7): endorsed after
/// one op and concurrent with the other, the readmit is refused and the
/// key it names still needs one.
#[test]
fn a_pre_fork_cosigner_must_precede_both_fork_ops() {
    let (out, r, after) = own_readmit(true);
    assert!(out.applied.contains(&r.hash()), "{out:?}");
    assert!(out.applied.contains(&after.hash()), "{out:?}");
    // Key 11 is endorsed after one fork op, so it is itself new after the
    // fork (rule 7, K4b.4): the readmit it signs is quarantined, never
    // applied, and the key it names still needs one.
    let (out, r, after) = own_readmit(false);
    assert!(needs_readmit(&out, &r), "{out:?}");
    assert!(needs_readmit(&out, &after), "{out:?}");
}

/// `t()`'s own readmit of key 20, signed by key 11, with a `DeviceRetired`
/// carrier of key 10 as the fork event: key 11's endorsement precedes the
/// carrier when `endorsed_first`, else follows it; key 20's endorsement
/// and the readmit name `fork`. Returns the outcome, the readmit and the
/// op of key 20 after it.
fn own_readmit_after_retirement(
    endorsed_first: bool,
    fork: [u8; 32],
) -> (MergeOutcome, VerifiedOp, VerifiedOp) {
    let tj = join_t();
    let e11_body = || GovernanceEntry::DeviceKey(endorse(11, 3, [0; 32]));
    let (first, second) = if endorsed_first {
        let e11 = op(&t(), 1, &[&tj], 1, e11_body());
        let ret = op(&t(), 2, &[&e11], 2, retire_10());
        (e11, ret)
    } else {
        let ret = op(&t(), 1, &[&tj], 1, retire_10());
        let e11 = op(&t(), 2, &[&ret], 2, e11_body());
        (ret, e11)
    };
    let dk = endorse_20(3, &[&second], 3, fork);
    let r = op_on(&t(), [11; 32], 0, &[&dk], 4, readmit(fork));
    let leave = GovernanceEntry::MemberLeft { heads: Vec::new() };
    let after = op_on(&t(), [20; 32], 0, &[&r], 5, leave);
    let out = outcome(&[&tj, &dk, &first, &second, &r, &after]);
    (out, r, after)
}

/// A `DeviceRetired` carrier is a fork event (ADR 0022 rule 7, plan E3.2
/// K4b Q7): an own-key readmit counts only when its co-signer was endorsed
/// in the carrier's strict causal past; endorsed after the carrier, the
/// readmit is refused and the key it names still needs one.
#[test]
fn a_pre_fork_cosigner_must_precede_a_retirement_carrier() {
    let (out, r, after) = own_readmit_after_retirement(true, retirement_10());
    assert!(out.applied.contains(&r.hash()), "{out:?}");
    assert!(out.applied.contains(&after.hash()), "{out:?}");
    // Key 11 is endorsed after the carrier, so it needs a readmit itself
    // (rule 7, K4b.4): the readmit it signs is quarantined, never applied.
    let (out, r, after) = own_readmit_after_retirement(false, retirement_10());
    assert!(needs_readmit(&out, &r), "{out:?}");
    assert!(needs_readmit(&out, &after), "{out:?}");
}

/// `t()` with keys 10, 11 and 12; keys 10 and 11 fork, the second fork
/// (ADR 0022 rule 7, plan E3.2 K4b Q9); key 12 signs seqs 0, 1 and 2, and
/// member 5 cites key 12's seq `cited`. Returns the outcome and key 12's
/// ops.
fn second_fork(cited: usize) -> (MergeOutcome, Vec<VerifiedOp>) {
    let tj = join_t();
    let device = |seq, deps: &[&VerifiedOp], key| {
        let e = GovernanceEntry::DeviceKey(endorse(key, u64::from(key), [0; 32]));
        op(&t(), seq, deps, seq, e)
    };
    let e11 = device(1, &[&tj], 11);
    let e12 = device(2, &[&e11], 12);
    let fork = |key: u8, name: &str, at: u64| op_on(&t(), [key; 32], 0, &[&e12], at, meta(name));
    let forks = [
        fork(10, "a", 3),
        fork(10, "b", 4),
        fork(11, "c", 5),
        fork(11, "d", 6),
    ];
    let mut k12: Vec<VerifiedOp> = Vec::new();
    for seq in 0..3u64 {
        let prev = k12.last().unwrap_or(&e12).clone();
        k12.push(op_on(&t(), [12; 32], seq, &[&prev], 7 + seq, meta("k12")));
    }
    let x = op(&pk(5), 0, &[&k12[cited]], 20, joined(5, None));
    let mut all = vec![&tj, &e11, &e12, &x];
    all.extend(forks.iter());
    all.extend(k12.iter());
    (outcome(&all), k12)
}

fn retired(out: &MergeOutcome, o: &VerifiedOp) -> bool {
    let reason = QuarantineReason::DeviceKey {
        refusal: KeyRefusal::Retired,
    };
    (out.quarantined.iter()).any(|(h, r)| *h == o.hash() && *r == reason)
}

/// A second fork retires every key of the member above its other-member
/// floor (ADR 0022 rule 7, plan E3.2 K4b Q9): key 12, never forked, keeps
/// what member 5 built on and loses the rest; a later op of member 5
/// citing more moves back exactly what it cites.
#[test]
fn a_second_fork_retires_every_key_above_its_other_member_floor() {
    let (out, k12) = second_fork(0);
    assert!(!retired(&out, &k12[0]), "{out:?}");
    assert!(retired(&out, &k12[1]) && retired(&out, &k12[2]), "{out:?}");
    let (out, k12) = second_fork(1);
    assert!(
        !retired(&out, &k12[0]) && !retired(&out, &k12[1]),
        "{out:?}"
    );
    assert!(retired(&out, &k12[2]), "{out:?}");
}

/// After `t()`'s first fork event (its retirement of key 10, carried after
/// key 11's endorsement), it endorses key 12 (ADR 0022 rule 7, plan E3.2
/// K4b.4); with `readmitted`, key 11, endorsed before the fork, co-signs a
/// readmit of key 12. Returns the ops through the snapshot head (the
/// readmit, else the endorsement), the head, key 12's op on it and key
/// 11's op after the fork (the readmit, else a leave).
fn new_key_after_fork(readmitted: bool) -> (Vec<VerifiedOp>, VerifiedOp, VerifiedOp, VerifiedOp) {
    let tj = join_t();
    let device = |key, inc, fork| GovernanceEntry::DeviceKey(endorse(key, inc, fork));
    let e11 = op(&t(), 1, &[&tj], 1, device(11, 3, [0; 32]));
    let ret = op(&t(), 2, &[&e11], 2, retire_10());
    // Key 12's endorsement names the retirement it follows (rule 7).
    let e12 = op(&t(), 3, &[&ret], 3, device(12, 4, retirement_10()));
    let leave = || GovernanceEntry::MemberLeft { heads: Vec::new() };
    let mut base = vec![tj, e11, ret.clone(), e12.clone()];
    let (head, k11) = if readmitted {
        let body = readmit_of(12, 4, retirement_10());
        // Key 11's op after the fork is the readmit it signs.
        let r = op_on(&t(), [11; 32], 0, &[&e12], 4, body);
        base.push(r.clone());
        (r.clone(), r)
    } else {
        (e12, op_on(&t(), [11; 32], 0, &[&ret], 5, leave()))
    };
    let k12 = op_on(&t(), [12; 32], 0, &[&head], 6, leave());
    (base, head, k12, k11)
}

/// A key endorsed after the member's first fork event needs an applied
/// `DeviceReadmit` (ADR 0022 rule 7, plan E3.2 K4b.4): without one its op
/// is quarantined `NeedsReadmit`; with key 11's own readmit it applies;
/// key 11, endorsed before the fork, is unaffected.
#[test]
fn a_key_endorsed_after_the_first_fork_needs_a_readmit() {
    for readmitted in [false, true] {
        let (base, _, k12, k11) = new_key_after_fork(readmitted);
        let mut all: Vec<&VerifiedOp> = base.iter().collect();
        all.extend([&k12, &k11]);
        let out = outcome(&all);
        assert!(out.applied.contains(&k11.hash()), "pre-fork key: {out:?}");
        assert_eq!(out.applied.contains(&k12.hash()), readmitted, "{out:?}");
        assert_eq!(needs_readmit(&out, &k12), !readmitted, "{out:?}");
    }
    // Rule 7 itself, not only the endorsement's own `forkEvidence`: a key
    // endorsed after the fork naming no fork event still needs a readmit.
    let tj = join_t();
    let device = |key, inc| GovernanceEntry::DeviceKey(endorse(key, inc, [0; 32]));
    let e11 = op(&t(), 1, &[&tj], 1, device(11, 3));
    let ret = op(&t(), 2, &[&e11], 2, retire_10());
    let e12 = op(&t(), 3, &[&ret], 3, device(12, 4));
    let leave = GovernanceEntry::MemberLeft { heads: Vec::new() };
    let k12 = op_on(&t(), [12; 32], 0, &[&e12], 4, leave);
    let out = outcome(&[&tj, &e11, &ret, &e12, &k12]);
    assert!(needs_readmit(&out, &k12), "zero evidence: {out:?}");
}

/// Resume stays exact for rule 7 (K4b.4): the snapshot keeps no fork
/// event's position, so a key neither readmitted nor the cert's is left to
/// the full merge (`DeviceJudged`), the new key and the pre-fork key
/// alike; a readmitted key resumes equal to the full merge (`resume`
/// checks every fate against it).
#[test]
fn resume_judges_rule_7_exactly() {
    let cert = cert(&pk(1), AdmissionMode::Open);
    for readmitted in [false, true] {
        let (base, head, k12, k11) = new_key_after_fork(readmitted);
        let with = |o: &VerifiedOp| base.iter().chain([o]).cloned().collect::<Vec<_>>();
        if !readmitted {
            let got = resume(&cert, &with(&k11), &[&head]);
            assert!(matches!(got, Err(NotOnTop::DeviceJudged(_))), "{got:?}");
        }
        let got = resume(&cert, &with(&k12), &[&head]);
        if readmitted {
            let out = got
                .unwrap_or_else(|e| panic!("{e:?}: k12 {:?} head {:?}", k12.hash(), head.hash()));
            assert!(out.applied.contains(&k12.hash()), "{out:?}");
        } else {
            assert!(matches!(got, Err(NotOnTop::DeviceJudged(_))), "{got:?}");
        }
    }
}

/// A readmit may answer a retirement (ADR 0022 rule 7, amended
/// 2026-11-23): a moderator's readmit naming the evidence of the
/// retirement its deps hold applies, and so does an own-key one signed by
/// a key endorsed before the retirement.
#[test]
fn a_readmit_may_answer_a_retirement() {
    let (out, after) = moderated(true, 10, true);
    assert!(out.applied.contains(&after.hash()), "moderator: {out:?}");
    let (out, r, after) = own_readmit_after_retirement(true, retirement_10());
    assert!(out.applied.contains(&r.hash()), "own key: {out:?}");
    assert!(out.applied.contains(&after.hash()), "own key: {out:?}");
}

/// A readmit whose `forkEvidence` is no fork event of its member held at
/// its deps is refused (ADR 0022 rule 7, amended 2026-11-23): bytes naming
/// no event, or a real retirement concurrent with the readmit, and the key
/// it names still needs a readmit.
#[test]
fn a_readmit_naming_no_held_fork_event_is_refused() {
    let (out, r, after) = own_readmit_after_retirement(true, [7; 32]);
    assert!(out.refused.contains(&r.hash()), "no event: {out:?}");
    assert!(needs_readmit(&out, &after), "no event: {out:?}");
    let (out, after) = moderated(true, 10, false);
    assert!(needs_readmit(&out, &after), "not held: {out:?}");
}

/// A key endorsed after the fork and readmitted by a moderator never
/// co-signs an own-key readmit (ADR 0022 rule 7: approvals never chain):
/// key 21's readmit of key 20 is refused and key 20 still needs one.
#[test]
fn a_readmitted_key_never_cosigns() {
    let (tj, dk, ret) = forked_member(true);
    let e21 = GovernanceEntry::DeviceKey(endorse(21, 3, retirement_10()));
    let e21 = op(&t(), 3, &[&dk], 3, e21);
    let r9 = op(&pk(1), 0, &[], 2, role(9, 50));
    let r8 = op(&pk(1), 1, &[&r9], 3, role(8, 10));
    let mj = op(&pk(3), 0, &[], 4, joined(3, None));
    let g = op(&pk(1), 2, &[&r8, &mj], 5, grant(pk(3), 9));
    let tg = op(&pk(1), 3, &[&g, &e21], 6, grant(t(), 8));
    let by_mod = op(
        &pk(3),
        1,
        &[&mj, &tg],
        7,
        readmit_of(21, 3, retirement_10()),
    );
    let own = op_on(&t(), [21; 32], 0, &[&by_mod], 8, readmit(retirement_10()));
    let after = op_on(&t(), [20; 32], 0, &[&own], 9, meta("after"));
    let all = [
        &tj, &ret, &dk, &e21, &r9, &r8, &mj, &g, &tg, &by_mod, &own, &after,
    ];
    let out = outcome(&all);
    assert!(out.applied.contains(&by_mod.hash()), "{out:?}");
    assert!(!out.applied.contains(&own.hash()), "{out:?}");
    assert!(out.refused.contains(&own.hash()), "{out:?}");
    assert!(needs_readmit(&out, &after), "{out:?}");
}
