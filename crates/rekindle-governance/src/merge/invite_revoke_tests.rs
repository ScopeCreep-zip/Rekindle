//! Plan SU1: an inviter that admits the join filling an invite's
//! `max_uses` revokes it citing that join (`bootstrap.rs`), and every
//! reader refuses a join outside that revoke's causal past, so a join
//! citing stale deps and ground to a lower hash never takes the use of a
//! join already admitted.

use proptest::prelude::*;
use rekindle_types::governance::{AdmissionMode, GovernanceEntry};
use rekindle_types::join_refusal::JoinRefusal;

use super::{invite, joined, pk};
use crate::fixture::{cert, op};
use crate::merge::merge_outcome;
use crate::op::VerifiedOp;

fn revoked() -> GovernanceEntry {
    GovernanceEntry::InviteRevoked { invite_id: [7; 16] }
}

/// Two joins on one single-use invite with equal deps: the inviter
/// admitted `first`, and `second` sorts before it (the lower hash). Once
/// the inviter's revoke (citing `first`) is merged, `first` stays admitted
/// and `second` is refused as `InviteUsedUp`, in either arrival order.
#[test]
fn an_admitted_join_keeps_a_single_use_invite_against_a_lower_hash() {
    let c0 = op(&pk(1), 0, &[], 0, joined(1, None));
    let c1 = op(&pk(1), 1, &[&c0], 1, invite(1));
    let a = op(&pk(2), 0, &[&c1], 2, joined(2, Some("code")));
    let b = op(&pk(3), 0, &[&c1], 3, joined(3, Some("code")));
    let (second, first) = if a.hash() < b.hash() { (a, b) } else { (b, a) };
    let cert = cert(&pk(1), AdmissionMode::Open);
    let before = merge_outcome(&cert, &[c0.clone(), c1.clone(), first.clone()]);
    assert!(
        before.state.members.contains_key(first.author()),
        "admitted"
    );
    let revoke = op(&pk(1), 2, &[&c1, &first], 4, revoked());
    let mut ops = vec![c0, c1, first.clone(), revoke, second.clone()];
    for _ in 0..2 {
        let outcome = merge_outcome(&cert, &ops);
        assert!(
            outcome.state.members.contains_key(first.author()),
            "the admitted join stays admitted"
        );
        assert!(!outcome.state.members.contains_key(second.author()));
        assert_eq!(
            outcome.join_refusals.get(&second.hash()),
            Some(&JoinRefusal::InviteUsedUp)
        );
        ops.reverse();
    }
}

/// A join concurrent with a manual revoke is refused (fail-closed: the
/// revoker never saw it), as `InviteNoLongerValid`: the revoke's past
/// holds no use.
#[test]
fn a_join_concurrent_with_a_manual_revoke_is_refused() {
    let c0 = op(&pk(1), 0, &[], 0, joined(1, None));
    let c1 = op(&pk(1), 1, &[&c0], 1, invite(2));
    let join = op(&pk(2), 0, &[&c1], 2, joined(2, Some("code")));
    let revoke = op(&pk(1), 2, &[&c1], 3, revoked());
    let outcome = merge_outcome(
        &cert(&pk(1), AdmissionMode::Open),
        &[c0, c1, join.clone(), revoke],
    );
    assert!(!outcome.state.members.contains_key(join.author()));
    assert_eq!(
        outcome.join_refusals.get(&join.hash()),
        Some(&JoinRefusal::InviteNoLongerValid)
    );
}

/// Fisher-Yates over `ops` from `seed` (xorshift).
fn shuffle(ops: &mut [VerifiedOp], seed: u64) {
    let mut x = seed | 1;
    for i in (1..ops.len()).rev() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        ops.swap(i, usize::try_from(x % (i as u64 + 1)).unwrap_or(0));
    }
}

proptest! {
    /// SU1: `n` joins with equal deps on an invite of `max_uses` uses; the
    /// inviter admitted `max_uses` of them, whichever (by `pick`), and its
    /// revoke cites them. Under any merge order every admitted join stays
    /// admitted and every other is refused `InviteUsedUp`.
    #[test]
    fn an_admitted_join_stays_admitted_under_any_merge_order(
        n in 2u8..=6,
        uses in 1u32..=5,
        pick in any::<u64>(),
        tags in any::<u64>(),
        order in any::<u64>(),
    ) {
        let max_uses = uses.min(u32::from(n) - 1);
        let c0 = op(&pk(1), 0, &[], 0, joined(1, None));
        let c1 = op(&pk(1), 1, &[&c0], 1, invite(max_uses));
        let mut joins: Vec<VerifiedOp> = (0..n)
            .map(|i| {
                let m = 10 + i;
                op(&pk(m), 0, &[&c1], tags.wrapping_add(u64::from(i)), joined(m, Some("code")))
            })
            .collect();
        shuffle(&mut joins, pick);
        let (admitted, others) = joins.split_at(usize::try_from(max_uses).unwrap_or(0));
        let mut cited: Vec<&VerifiedOp> = vec![&c1];
        cited.extend(admitted.iter());
        let revoke = op(&pk(1), 2, &cited, 9, revoked());
        let mut ops = vec![c0.clone(), c1.clone(), revoke];
        ops.extend(joins.iter().cloned());
        shuffle(&mut ops, order);
        let outcome = merge_outcome(&cert(&pk(1), AdmissionMode::Open), &ops);
        for join in admitted {
            prop_assert!(outcome.state.members.contains_key(join.author()));
        }
        for join in others {
            prop_assert!(!outcome.state.members.contains_key(join.author()));
            prop_assert_eq!(
                outcome.join_refusals.get(&join.hash()),
                Some(&JoinRefusal::InviteUsedUp)
            );
        }
    }
}
