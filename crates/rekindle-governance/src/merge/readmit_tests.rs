//! `DeviceReadmit` authority (ADR 0022 rules 7 and 8, plan E3.2 K4b BUG
//! and Q6): a readmit enters the device view only once applied, and a
//! moderator's is judged at its causal position, as a kick is.

use rekindle_secrets::ed25519_dalek::{Signer, SigningKey};
use rekindle_types::governance::{
    DeviceEndorsement, DeviceReadmit, DeviceRetired, GovernanceEntry,
};
use rekindle_types::id::{PseudonymKey, RoleId};

use super::judge::tests::outcome;
use super::op_proptests::{joined, pk, role};
use super::{MergeOutcome, QuarantineReason};
use crate::device::{device_set_hash, endorsement_signing_bytes, retirement_signing_bytes};
use crate::device_judge::KeyRefusal;
use crate::fixture::{meta, op, op_on, FIXTURE_ANCHOR};
use crate::op::VerifiedOp;

const FORK: [u8; 32] = [7; 32];
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

fn readmit() -> GovernanceEntry {
    GovernanceEntry::DeviceReadmit(DeviceReadmit {
        member: t(),
        member_record: RECORD.into(),
        new_key: [20; 32],
        incarnation: 2,
        fork_evidence: FORK,
    })
}

fn grant(target: PseudonymKey, id: u8) -> GovernanceEntry {
    GovernanceEntry::RoleAssignment {
        target,
        role_id: RoleId([id; 16]),
    }
}

/// `t()` joined with key 10, and its endorsement of key 20 answering a
/// fork: key 20 needs a readmit.
fn forked_member() -> (VerifiedOp, VerifiedOp) {
    let join = GovernanceEntry::MemberJoined {
        member_record: RECORD.into(),
        member_record_owner: "owner".into(),
        invite_proof: None,
        owner_proof: vec![0; 64],
        first_device: Some(endorse(10, 1, [0; 32])),
    };
    let tj = op(&t(), 0, &[], 0, join);
    let dk = op(
        &t(),
        1,
        &[&tj],
        1,
        GovernanceEntry::DeviceKey(endorse(20, 2, FORK)),
    );
    (tj, dk)
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
    let (tj, dk) = forked_member();
    let x = op(&pk(5), 0, &[], 2, joined(5, None));
    let r = op(&pk(5), 1, &[&x, &dk], 3, readmit());
    let after = op_on(&t(), [20; 32], 0, &[&r], 4, meta("after"));
    let out = outcome(&[&tj, &dk, &x, &r, &after]);
    assert!(out.refused.contains(&r.hash()), "the readmit is refused");
    assert!(needs_readmit(&out, &after), "{out:?}");
    assert!(!out.applied.contains(&after.hash()));
}

/// Creator 1 defines role 9 (`KICK_MEMBERS`, rank 50) and grants it to
/// member 3, and gives `t()` role 8 at `t_rank`; member 3's readmit of
/// `t()`'s key 20 cites the grant when `cites_grant`. Returns the outcome
/// and the op of key 20 after the readmit.
fn moderated(cites_grant: bool, t_rank: u32) -> (MergeOutcome, VerifiedOp) {
    let (tj, dk) = forked_member();
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
    let r = op(&pk(3), 1, &deps, 7, readmit());
    let after = op_on(&t(), [20; 32], 0, &[&r], 8, meta("after"));
    (
        outcome(&[&tj, &dk, &r9, &r8, &mj, &g, &tg, &r, &after]),
        after,
    )
}

/// A moderator's readmit is judged against the power state in its causal
/// past (ADR 0022 rule 8, plan E3.2 K4b Q6), not at genesis: with the
/// grant in its deps it applies and the key is effective; with the grant
/// concurrent, or the member at the moderator's rank, it is refused.
#[test]
fn a_moderator_readmit_is_judged_at_its_causal_rank() {
    let (out, after) = moderated(true, 10);
    assert!(out.applied.contains(&after.hash()), "{out:?}");
    let (out, after) = moderated(false, 10);
    assert!(needs_readmit(&out, &after), "grant concurrent: {out:?}");
    let (out, after) = moderated(true, 50);
    assert!(needs_readmit(&out, &after), "not outranked: {out:?}");
}

/// `t()`'s own readmit of key 20, signed by key 11, whose endorsement both
/// ops of key 10's fork cite when `both`, else only the first. Returns the
/// outcome, the readmit and the op of key 20 after it.
fn own_readmit(both: bool) -> (MergeOutcome, VerifiedOp, VerifiedOp) {
    let (tj, dk) = forked_member();
    let e11 = op(
        &t(),
        2,
        &[&dk],
        2,
        GovernanceEntry::DeviceKey(endorse(11, 3, [0; 32])),
    );
    let f1 = op_on(&t(), [10; 32], 0, &[&e11], 3, meta("a"));
    let f2_deps: [&VerifiedOp; 1] = if both { [&e11] } else { [&dk] };
    let f2 = op_on(&t(), [10; 32], 0, &f2_deps, 4, meta("b"));
    let r = op_on(&t(), [11; 32], 0, &[&f1, &f2], 5, readmit());
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
    let (out, r, after) = own_readmit(false);
    assert!(out.refused.contains(&r.hash()), "{out:?}");
    assert!(needs_readmit(&out, &after), "{out:?}");
}

/// `t()`'s retirement of key 10 (incarnation 1) above seq 0.
fn retire_10() -> GovernanceEntry {
    let mut r = DeviceRetired {
        device_key: [10; 32],
        incarnation: 1,
        last_seq: 0,
        retirement_sig: Vec::new(),
    };
    let bytes = retirement_signing_bytes(FIXTURE_ANCHOR, RECORD, &r);
    r.retirement_sig = (SigningKey::from_bytes(&[2; 32]).sign(&bytes).to_bytes()).to_vec();
    GovernanceEntry::DeviceRetired(r)
}

/// `t()`'s own readmit of key 20, signed by key 11, with a `DeviceRetired`
/// carrier of key 10 as the fork event: key 11's endorsement precedes the
/// carrier when `endorsed_first`, else follows it. Returns the outcome,
/// the readmit and the op of key 20 after it.
fn own_readmit_after_retirement(endorsed_first: bool) -> (MergeOutcome, VerifiedOp, VerifiedOp) {
    let (tj, dk) = forked_member();
    let e11_body = || GovernanceEntry::DeviceKey(endorse(11, 3, [0; 32]));
    let (first, second) = if endorsed_first {
        let e11 = op(&t(), 2, &[&dk], 2, e11_body());
        let ret = op(&t(), 3, &[&e11], 3, retire_10());
        (e11, ret)
    } else {
        let ret = op(&t(), 2, &[&dk], 2, retire_10());
        let e11 = op(&t(), 3, &[&ret], 3, e11_body());
        (ret, e11)
    };
    let r = op_on(&t(), [11; 32], 0, &[&second], 4, readmit());
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
    let (out, r, after) = own_readmit_after_retirement(true);
    assert!(out.applied.contains(&r.hash()), "{out:?}");
    assert!(out.applied.contains(&after.hash()), "{out:?}");
    let (out, r, after) = own_readmit_after_retirement(false);
    assert!(out.refused.contains(&r.hash()), "{out:?}");
    assert!(needs_readmit(&out, &after), "{out:?}");
}

/// `t()` with keys 10, 11 and 12; keys 10 and 11 fork, the second fork
/// (ADR 0022 rule 7, plan E3.2 K4b Q9); key 12 signs seqs 0, 1 and 2, and
/// member 5 cites key 12's seq `cited`. Returns the outcome and key 12's
/// ops.
fn second_fork(cited: usize) -> (MergeOutcome, Vec<VerifiedOp>) {
    let (tj, _) = forked_member();
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
