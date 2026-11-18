//! Using a checkpoint (plan E3.2 S7c.5): what a reader that skipped the
//! cut reads from the cut index, so a hint above height 1 is usable.

use std::collections::HashSet;

use rekindle_types::governance::{AdmissionMode, Checkpoint, GovernanceEntry, OpHash};
use rekindle_types::id::PseudonymKey;

use super::op_proptests::{community, find, head, pk, role};
use super::{merge_cut, merge_from, merge_outcome};
use crate::checkpoint::{
    causal_past, eligible_set_hash, frontier_hash, state_preimage, state_root_of, CutSnapshot,
};
use crate::fixture::{cert, op};
use crate::op::VerifiedOp;
use crate::power::{class_of, OpClass};

fn body(
    height: u64,
    prev: Option<&VerifiedOp>,
    cut: &[&VerifiedOp],
    e: &[PseudonymKey],
) -> Checkpoint {
    let heads: Vec<OpHash> = cut.iter().map(|o| o.hash()).collect();
    Checkpoint {
        height,
        round: 0,
        prev: prev.map(|p| p.op_ref("VLD0:fixture")),
        cut: cut.iter().map(|o| o.op_ref("VLD0:fixture")).collect(),
        frontier_hash: frontier_hash(&heads),
        dir_root: [0; 32],
        dir_size: 0,
        state_root: [0; 32],
        state_size: 0,
        mek_generation: 1,
        eligible_set_hash: eligible_set_hash(e),
        pages: Vec::new(),
    }
}

/// `community()` (members 1–4), the creator's height-1 checkpoint `cp1`
/// over its head, `x` past `cp1` (the height-2 cut's head), and three
/// members' height-2 endorsements over `x` naming `cp1`, which is in the
/// height-2 cut.
fn world() -> (Vec<VerifiedOp>, VerifiedOp, Vec<VerifiedOp>) {
    let mut ops = community();
    let c5 = head(&ops).clone();
    let cp1 = op(
        &pk(1),
        6,
        &[&c5],
        100,
        GovernanceEntry::Checkpoint(body(1, None, &[&c5], &[pk(1)])),
    );
    let x = op(&pk(3), 1, &[&cp1], 101, role(8, 1));
    let members: Vec<PseudonymKey> = (1..=4).map(pk).collect();
    let cp2 = body(2, Some(&cp1), &[&x], &members);
    let votes: Vec<VerifiedOp> = [(1, 7), (2, 1), (4, 1)]
        .into_iter()
        .map(|(a, seq)| {
            op(
                &pk(a),
                seq,
                &[&x],
                200 + u64::from(a),
                GovernanceEntry::Checkpoint(cp2.clone()),
            )
        })
        .collect();
    ops.extend([cp1, x.clone()]);
    (ops, x, votes)
}

/// The snapshot at the cut whose head is `head`, its power ops and `rest`.
fn split(all: &[VerifiedOp], head: &VerifiedOp) -> (CutSnapshot, Vec<VerifiedOp>, Vec<VerifiedOp>) {
    let cert = cert(&pk(1), AdmissionMode::Open);
    let cut_ops = causal_past(all, &[head.op_ref("VLD0:fixture")]);
    let (state, cut) = merge_cut(&cert, &cut_ops);
    let bytes = state_preimage(&state, &cut);
    let mut b = body(2, None, &[head], &[]);
    b.state_root = state_root_of(&bytes);
    b.state_size = u32::try_from(bytes.len()).expect("fits");
    let snap = CutSnapshot::decode(&b, &bytes).expect("a canonical snapshot");
    let in_cut: HashSet<OpHash> = cut_ops.iter().map(VerifiedOp::hash).collect();
    let power = (cut_ops.iter())
        .filter(|o| class_of(o.body()) == OpClass::Power)
        .cloned()
        .collect();
    let rest = (all.iter())
        .filter(|o| !in_cut.contains(&o.hash()))
        .cloned()
        .collect();
    (snap, power, rest)
}

/// S7c.4a left a `Checkpoint` op whose `prevCheckpoint` is in the cut
/// `Undecidable`, so every endorsement of a hint above height 1 sent its
/// reader to full replay; the cut index's applied checkpoints decide it,
/// exactly as the full merge does.
#[test]
fn a_hints_endorsements_naming_a_checkpoint_in_the_cut_are_decided() {
    let cert = cert(&pk(1), AdmissionMode::Open);
    let (mut all, x, votes) = world();
    all.extend(votes.iter().cloned());
    let (snap, power, rest) = split(&all, &x);
    let resumed = merge_from(&cert, &snap, &power, &rest).expect("on top of the height-2 cut");
    let full = merge_outcome(&cert, &all);
    assert_eq!(resumed.state, full.state);
    for op in &rest {
        let h = op.hash();
        assert_eq!(
            resumed.applied.contains(&h),
            full.applied.contains(&h),
            "a post-cut fate"
        );
    }
    for vote in &votes {
        assert!(
            resumed.applied.contains(&vote.hash()),
            "the endorsement applies"
        );
    }
}

/// A post-cut op's lamport is one above its deps': the cut index gives the
/// lamport of each author's top cut op (every head among them), and no
/// other cut op, so a reader verifies an op citing the heads without the
/// cut and sends one citing an older cut op to the full merge.
#[test]
fn the_cut_index_gives_the_lamport_of_each_authors_top_op_only() {
    let (all, x, _) = world();
    let (snap, _, _) = split(&all, &x);
    let at = x.op_ref("VLD0:fixture");
    let facts = snap
        .cut
        .top_facts(&at)
        .expect("the head is its author's top op");
    assert_eq!((facts.seq, facts.lamport), (x.op().seq, x.lamport()));
    let older = find(&all, 1, 0).op_ref("VLD0:fixture");
    assert!(
        snap.cut.top_facts(&older).is_none(),
        "not its author's top op"
    );
}
