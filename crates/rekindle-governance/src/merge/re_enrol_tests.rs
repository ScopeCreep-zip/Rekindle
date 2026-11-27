//! The exempt replacement (ADR 0022 rules 4 and 7 as amended in plan E3.2
//! K4b.4 R3; plan E3.2 S7d-keys.10): a `DeviceReEnrol`, signed by the new
//! key at its seq 0, carries the retirement of the copied key and the new
//! key's endorsement. Its key needs no co-signer iff the re-enrolment's
//! strict past holds no fork event of the member; of concurrent ones only
//! the lowest `(lamport, op_hash)` is exempt; any other needs a
//! `DeviceReadmit`, as a post-fork key does.

use rekindle_types::governance::{DeviceReEnrol, GovernanceEntry};

use super::judge::tests::outcome;
use super::readmit_tests::{endorse, join_t, needs_readmit, retired, retired_10, t};
use crate::fixture::{op, op_on};
use crate::op::VerifiedOp;

/// `t()`'s re-enrolment: key 10 retired above seq 0, `[key; 32]` endorsed
/// at `incarnation` answering no fork, on `[key; 32]`'s seq 0.
fn re_enrol(key: u8, incarnation: u64, deps: &[&VerifiedOp], at: u64) -> VerifiedOp {
    let body = GovernanceEntry::DeviceReEnrol(DeviceReEnrol {
        retired: retired_10(),
        endorsement: endorse(key, incarnation, [0; 32]),
    });
    op_on(&t(), [key; 32], 0, deps, at, body)
}

/// `[key; 32]`'s op at `seq` on `deps`.
fn later(key: u8, seq: u64, deps: &[&VerifiedOp], at: u64) -> VerifiedOp {
    let leave = GovernanceEntry::MemberLeft { heads: Vec::new() };
    op_on(&t(), [key; 32], seq, deps, at, leave)
}

/// R3, exempt: a re-enrolment with no fork event in its strict past makes
/// its new key effective with no co-signer, for itself and every later op
/// of the key (without R3 the later op is `NeedsReadmit`).
#[test]
fn a_re_enrolment_with_no_fork_event_in_its_past_needs_no_co_signer() {
    let tj = join_t();
    let x = re_enrol(20, 2, &[&tj], 1);
    let k20 = later(20, 1, &[&x], 2);
    let out = outcome(&[&tj, &x, &k20]);
    assert!(out.applied.contains(&x.hash()), "{out:?}");
    assert!(out.applied.contains(&k20.hash()), "{out:?}");
    assert!(!needs_readmit(&out, &k20), "{out:?}");
}

/// R3, not exempt: a re-enrolment whose strict past holds a fork event of
/// the member (here an earlier retirement) needs a co-signer, and so does
/// every later op of its key.
#[test]
fn a_re_enrolment_after_a_fork_event_needs_a_readmit() {
    let tj = join_t();
    let ret = op(
        &t(),
        1,
        &[&tj],
        1,
        GovernanceEntry::DeviceRetired(retired_10()),
    );
    let x = re_enrol(20, 2, &[&ret], 2);
    let k20 = later(20, 1, &[&x], 3);
    let out = outcome(&[&tj, &ret, &x, &k20]);
    assert!(needs_readmit(&out, &x), "{out:?}");
    assert!(needs_readmit(&out, &k20), "{out:?}");
}

/// R3, concurrent: both copies of a cloned key re-enrol, each with no fork
/// event in its strict past. Each re-enrolment alone stands; an op whose
/// deps hold both admits only the lowest `(lamport, op_hash)` one's key,
/// the other's needs a readmit; and the two retirements count as one fork,
/// never a second (no key of the member is retired by them).
#[test]
fn both_copies_of_a_cloned_key_re_enrol_and_count_as_one_fork() {
    let tj = join_t();
    let x = re_enrol(20, 2, &[&tj], 1);
    let y = re_enrol(21, 3, &[&tj], 2);
    let k20 = later(20, 1, &[&x, &y], 3);
    let k21 = later(21, 1, &[&x, &y], 4);
    let all = [&tj, &x, &y, &k20, &k21];
    let out = outcome(&all);
    assert!(out.applied.contains(&x.hash()), "{out:?}");
    assert!(out.applied.contains(&y.hash()), "{out:?}");
    // Both cite only the join, so the op hash breaks the lamport tie.
    let (low, high) = if x.stamp() < y.stamp() {
        (&k20, &k21)
    } else {
        (&k21, &k20)
    };
    assert!(out.applied.contains(&low.hash()), "lowest: {out:?}");
    assert!(needs_readmit(&out, high), "the other copy: {out:?}");
    assert!(!(all.iter()).any(|o| retired(&out, o)), "one fork: {out:?}");
    let mut reversed = all;
    reversed.reverse();
    assert_eq!(outcome(&reversed).state, out.state, "order-free");
}
