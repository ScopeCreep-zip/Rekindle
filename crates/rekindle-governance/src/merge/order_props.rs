//! Op-merge order properties (plan E3.2; C9.4 S4, S5, S6): no op at any
//! lamport becomes the creator, and one op set reaches one state in any
//! delivery order.

use proptest::prelude::*;
use rekindle_types::governance::AdmissionMode;

use super::super::{merge, merge_outcome};
use super::{community, find, head, kick, pk, role};
use crate::fixture::{cert, meta, op, op_at};

proptest! {
    /// S4: an op at any lamport, 0 included, never becomes the creator, and
    /// a forger's power op is refused.
    #[test]
    fn no_op_at_any_lamport_becomes_the_creator(lamport in any::<u64>()) {
        let ops = community();
        let forged = op_at(&pk(9), 0, &[], 80, lamport, role(9, 100));
        let mut set = vec![forged.clone()];
        set.extend(ops);
        let outcome = merge_outcome(&cert(&pk(1), AdmissionMode::Open), &set);
        prop_assert_eq!(outcome.state.creators, std::collections::BTreeSet::from([pk(1)]));
        prop_assert!(!outcome.applied.contains(&forged.hash()));
    }

    /// S5/S6: two nodes holding one op set, received in any order, reach
    /// equal state.
    #[test]
    fn one_op_set_one_state(seed in any::<u64>()) {
        let mut ops = community();
        let k2 = op(&pk(2), 1, &[head(&ops), find(&ops, 2, 0)], 30, kick(3, 1));
        let k3 = op(&pk(3), 1, &[head(&ops), find(&ops, 3, 0)], 31, kick(4, 1));
        let m = op(&pk(4), 1, &[head(&ops), find(&ops, 4, 0)], 32, meta("m"));
        ops.extend([k2, k3, m]);
        let mut shuffled = ops.clone();
        let len = shuffled.len();
        let mut x = seed | 1;
        for i in (1..len).rev() {
            x ^= x << 13; x ^= x >> 7; x ^= x << 17;
            shuffled.swap(i, usize::try_from(x % (i as u64 + 1)).unwrap_or(0));
        }
        let cert = cert(&pk(1), AdmissionMode::Open);
        prop_assert_eq!(merge(&cert, &ops), merge(&cert, &shuffled));
    }
}
