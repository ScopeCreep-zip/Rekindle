use proptest::prelude::*;
use rekindle_types::id::PseudonymKey;

use super::prepare;
use crate::fixture::{meta, op};
use crate::op::VerifiedOp;

/// A DAG of `n` ops, op `k` citing the earlier ops `picks` selects.
fn dag(n: usize, picks: &[u64]) -> Vec<VerifiedOp> {
    let mut ops: Vec<VerifiedOp> = Vec::new();
    for k in 0..n {
        let bits = picks[k % picks.len()].rotate_left(u32::try_from(k % 64).unwrap_or(0));
        let deps: Vec<&VerifiedOp> = (0..k)
            .filter(|&d| bits & (1 << ((k + 3 * d) % 64)) != 0 && k - d <= 70)
            .map(|d| &ops[d])
            .collect();
        let author = PseudonymKey([u8::try_from(k % 251).unwrap_or(0); 32]);
        let next = op(
            &author,
            u64::try_from(k).unwrap_or(0),
            &deps,
            k as u64,
            meta("m"),
        );
        ops.push(next);
    }
    ops
}

proptest! {
    /// R4-5: rows built on demand, in any query order, with the
    /// contained-dep skip, equal causal reachability over the deps.
    #[test]
    fn ancestry_is_reachability_in_any_query_order(
        n in 1usize..160,
        picks in proptest::collection::vec(any::<u64>(), 1..8),
        order in proptest::collection::vec(any::<u32>(), 1..400),
    ) {
        let ops = dag(n, &picks);
        let creator = PseudonymKey([0; 32]);
        let cert = crate::fixture::cert(&creator, rekindle_types::governance::AdmissionMode::Open);
        let set = prepare(&cert, &ops, &std::collections::BTreeSet::new());
        let at = |h| set.ops.iter().position(|o| o.hash() == h);
        let mut reach = vec![vec![false; set.ops.len()]; set.ops.len()];
        for i in 0..set.ops.len() {
            for d in &set.ops[i].op().deps {
                let d = at(d.op_hash).unwrap_or(usize::MAX);
                prop_assert!(d < i);
                reach[i][d] = true;
                let below = reach[d].clone();
                for (r, b) in reach[i].iter_mut().zip(below) {
                    *r |= b;
                }
            }
        }
        let m = set.ops.len();
        for q in order {
            let q = q as usize;
            let (i, j) = (q % m, (q / m) % m);
            prop_assert_eq!(set.past.contains(i, j), reach[i][j], "contains({}, {})", i, j);
        }
    }

    /// PF1: the anchor recurrence over the deps equals the latest
    /// marked op that `contains` finds in each op's causal past.
    #[test]
    fn latest_in_past_is_the_latest_marked_op_in_the_past(
        n in 1usize..160,
        picks in proptest::collection::vec(any::<u64>(), 1..8),
        marks in proptest::collection::vec(any::<u8>(), 1..16),
    ) {
        let ops = dag(n, &picks);
        let creator = PseudonymKey([0; 32]);
        let cert = crate::fixture::cert(&creator, rekindle_types::governance::AdmissionMode::Open);
        let set = prepare(&cert, &ops, &std::collections::BTreeSet::new());
        let m = set.ops.len();
        let order: Vec<usize> =
            (0..m).filter(|&i| marks[i % marks.len()] % 3 == 0).collect();
        let latest = set.past.latest_in_past(&super::super::positions(m, &order));
        for (i, got) in latest.into_iter().enumerate() {
            let want = order.iter().rposition(|&p| set.past.contains(i, p));
            prop_assert_eq!(got, want, "op {}", i);
        }
    }
}
