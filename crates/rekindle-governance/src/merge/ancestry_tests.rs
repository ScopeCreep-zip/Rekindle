//! Quarantined ops as ancestry, the authority cascade and the one-way rule
//! for refusals (ADR 0022 rule 5, plan E3.2 S7d-keys.2a), on today's
//! author-keyed forks.

use proptest::prelude::*;
use rekindle_types::governance::{AdmissionMode, GovernanceEntry};

use super::op_proptests::{assign, community, find, head, kick, pk};
use super::{merge_outcome, MergeOutcome, QuarantineReason};
use crate::fixture::{cert, meta, op};
use crate::op::VerifiedOp;

fn outcome(ops: &[VerifiedOp]) -> MergeOutcome {
    merge_outcome(&cert(&pk(1), AdmissionMode::Open), ops)
}

/// `victim` is quarantined because `cause` was.
fn because(outcome: &MergeOutcome, victim: &VerifiedOp, cause: &VerifiedOp) -> bool {
    outcome.quarantined.contains(&(
        victim.hash(),
        QuarantineReason::AuthorityQuarantined {
            cause: cause.hash(),
        },
    ))
}

fn applied(outcome: &MergeOutcome, op: &VerifiedOp) -> bool {
    outcome.applied.contains(&op.hash())
}

fn refused(outcome: &MergeOutcome, op: &VerifiedOp) -> bool {
    outcome.refused.contains(&op.hash())
}

/// `author`'s other op at `seq`: the fork that quarantines its op there.
fn late_fork(ops: &[VerifiedOp], author: u8, seq: u64, lamport: u64) -> VerifiedOp {
    op(
        &pk(author),
        seq,
        &[find(ops, author, seq - 1)],
        lamport,
        meta("the fork"),
    )
}

/// The community with member 4 granted role 5 by the creator (it may set
/// the meta, and manager 2 outranks it).
fn with_member_role() -> Vec<VerifiedOp> {
    let mut ops = community();
    let grant = op(&pk(1), 6, &[head(&ops)], 9, assign(4, 5));
    ops.push(grant);
    ops
}

#[test]
fn a_quarantined_op_stays_ancestry_and_its_dependants_resolve() {
    let mut ops = community();
    let f1 = op(&pk(2), 1, &[head(&ops), find(&ops, 2, 0)], 60, meta("one"));
    let f2 = op(&pk(2), 1, &[head(&ops), find(&ops, 2, 0)], 61, meta("two"));
    let x = op(&pk(3), 1, &[&f1, find(&ops, 3, 0)], 62, meta("x"));
    ops.extend([f1.clone(), f2.clone(), x.clone()]);
    let out = outcome(&ops);
    assert_eq!(out.equivocators, vec![(pk(2), pk(2).0, 1)]);
    assert!(out.pending.is_empty(), "nothing waits on a quarantined op");
    assert!(applied(&out, &x));
    for f in [&f1, &f2] {
        assert!(!applied(&out, f) && !refused(&out, f));
        assert!(!out.quarantined.iter().any(|(h, _)| *h == f.hash()));
    }
}

#[test]
fn a_late_fork_re_validates_a_role_grant_s_dependants_and_shows_why() {
    let mut ops = community();
    let grant = op(&pk(1), 6, &[head(&ops)], 60, assign(4, 5));
    let m = op(&pk(4), 1, &[&grant, find(&ops, 4, 0)], 61, meta("m"));
    ops.extend([grant.clone(), m.clone()]);
    assert!(applied(&outcome(&ops), &m), "authority from the grant");
    ops.push(late_fork(&ops, 1, 6, 62));
    let out = outcome(&ops);
    assert!(because(&out, &m, &grant));
    assert!(!applied(&out, &m) && !refused(&out, &m) && out.pending.is_empty());
}

#[test]
fn an_op_with_independent_authority_stays_applied_after_a_late_fork() {
    let mut ops = with_member_role();
    let grant = op(&pk(1), 7, &[head(&ops)], 60, assign(4, 6));
    let m = op(&pk(4), 1, &[&grant, find(&ops, 4, 0)], 61, meta("m"));
    ops.extend([grant.clone(), m.clone()]);
    assert!(applied(&outcome(&ops), &grant));
    ops.push(late_fork(&ops, 1, 7, 62));
    let out = outcome(&ops);
    assert!(applied(&out, &m), "role 5 from the creator still holds");
    assert!(!out.quarantined.iter().any(|(h, _)| *h == m.hash()));
}

/// Manager 2 kicks member 4 (role 5) at seq 1; member 4's concurrent seq-1
/// op is masked by the kick; then the kick is quarantined by 2's fork.
fn kicked() -> (Vec<VerifiedOp>, VerifiedOp, VerifiedOp, VerifiedOp) {
    let mut ops = with_member_role();
    let k = op(&pk(2), 1, &[head(&ops), find(&ops, 2, 0)], 60, kick(4, 1));
    let x = op(&pk(4), 1, &[head(&ops), find(&ops, 4, 0)], 61, meta("x"));
    ops.extend([k.clone(), x.clone()]);
    let late = late_fork(&ops, 2, 1, 62);
    ops.push(late.clone());
    (ops, k, x, late)
}

#[test]
fn a_late_quarantined_ban_keeps_the_targets_refused_ops_refused_until_re_sent() {
    let (ops, k, x, _) = kicked();
    let before: Vec<VerifiedOp> = ops.iter().take(ops.len() - 1).cloned().collect();
    assert!(outcome(&before).quarantined.contains(&(
        x.hash(),
        QuarantineReason::AfterAuthorRemoval { cause: k.hash() }
    )));
    let out = outcome(&ops);
    assert!(refused(&out, &x), "the kick's refusal is one-way");
    assert!(!applied(&out, &x));
    assert!(out.state.members.contains_key(&pk(4)), "the kick is undone");
}

#[test]
fn a_re_sent_op_with_the_cause_in_its_deps_is_applied() {
    let (mut ops, _, x, late) = kicked();
    let again = op(&pk(4), 2, &[&x, &late], 63, meta("again"));
    ops.push(again.clone());
    let out = outcome(&ops);
    assert!(applied(&out, &again));
    assert!(refused(&out, &x));
}

#[test]
fn a_late_quarantined_timeout_lifts_no_refusal() {
    let mut ops = community();
    let t = op(
        &pk(1),
        6,
        &[head(&ops)],
        60,
        GovernanceEntry::TimeoutEntry {
            target: pk(4),
            duration_seconds: 60,
            reason: None,
            started_at: 0,
        },
    );
    // Member 4 holds no role: its grant is refused, whatever the timeout.
    let y = op(&pk(4), 1, &[&t, find(&ops, 4, 0)], 61, assign(3, 6));
    let z = op(&pk(3), 1, &[&t, find(&ops, 3, 0)], 62, meta("z"));
    ops.extend([t, y.clone(), z.clone()]);
    let before = outcome(&ops);
    assert!(before.state.timeouts.contains_key(&pk(4)));
    ops.push(late_fork(&ops, 1, 6, 63));
    let out = outcome(&ops);
    assert!(
        !out.state.timeouts.contains_key(&pk(4)),
        "the timeout is undone"
    );
    assert!(refused(&before, &y) && refused(&out, &y));
    assert!(applied(&before, &z) && applied(&out, &z));
}

#[test]
fn the_cause_is_the_lowest_quarantined_op_in_the_authority_path() {
    let mut ops = community();
    let g1 = op(&pk(1), 6, &[head(&ops)], 60, assign(4, 5));
    let g2 = op(&pk(1), 7, &[&g1], 61, assign(4, 6));
    let m = op(&pk(4), 1, &[&g2, find(&ops, 4, 0)], 62, meta("m"));
    ops.extend([g1.clone(), g2.clone(), m.clone()]);
    ops.push(late_fork(&ops, 1, 6, 63));
    let out = outcome(&ops);
    assert!(because(&out, &m, &g1), "both grants are in its path");
    assert!(!because(&out, &m, &g2));
}

proptest! {
    /// The verdicts are equal for every arrival order (D15).
    #[test]
    fn ancestry_verdicts_are_equal_for_every_arrival_order(seed in any::<u64>()) {
        let (mut ops, _, x, late) = kicked();
        let grant = op(&pk(1), 7, &[find(&ops, 1, 6)], 64, assign(3, 6));
        let fork = late_fork(&ops, 1, 7, 65);
        let m = op(&pk(3), 1, &[&grant, find(&ops, 3, 0)], 65, meta("m"));
        let again = op(&pk(4), 2, &[&x, &late], 66, meta("again"));
        ops.extend([grant, fork, m, again]);
        let mut shuffled = ops.clone();
        let len = shuffled.len();
        let mut s = seed | 1;
        for i in (1..len).rev() {
            s ^= s << 13; s ^= s >> 7; s ^= s << 17;
            shuffled.swap(i, usize::try_from(s % (i as u64 + 1)).unwrap_or(0));
        }
        let (a, b) = (outcome(&ops), outcome(&shuffled));
        prop_assert!(refused(&a, &x) && a.pending.is_empty());
        prop_assert_eq!(a, b);
    }
}
