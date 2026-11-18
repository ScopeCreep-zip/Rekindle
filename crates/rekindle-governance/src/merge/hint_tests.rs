//! Proposal and endorsement (plan E3.2 S7c.4): usable hints, write-once
//! endorsements per (height, round), rounds without finality and
//! equivocation.

use rekindle_types::governance::{AdmissionMode, Checkpoint, GovernanceEntry, OpHash};
use rekindle_types::id::PseudonymKey;

use super::op_proptests::{community, head, pk, role};
use super::{merge_outcome, MergeOutcome, QuarantineReason};
use crate::checkpoint::{checkpoint_digest, eligible_set_hash, endorsement, frontier_hash, Replay};
use crate::fixture::{cert, meta, op};
use crate::op::VerifiedOp;

struct World {
    ops: Vec<VerifiedOp>,
    cp1: VerifiedOp,
    /// Two ops past the first cut, each the head of a different cut.
    x: VerifiedOp,
    y: VerifiedOp,
}

/// `community()` (members 1–4), the creator's height-1 checkpoint over its
/// head, and two concurrent ops past it.
fn world() -> World {
    let mut ops = community();
    let c5 = head(&ops).clone();
    let cp1 = op(
        &pk(1),
        6,
        &[&c5],
        100,
        GovernanceEntry::Checkpoint(body(1, None, &[&c5], &[pk(1)])),
    );
    let x = op(&pk(3), 1, &[&c5], 101, role(8, 1));
    let y = op(&pk(2), 1, &[&c5], 102, role(9, 1));
    ops.extend([cp1.clone(), x.clone(), y.clone()]);
    World { ops, cp1, x, y }
}

fn body(
    height: u64,
    prev: Option<&VerifiedOp>,
    cut: &[&VerifiedOp],
    electorate: &[PseudonymKey],
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
        eligible_set_hash: eligible_set_hash(electorate),
        pages: Vec::new(),
    }
}

fn members() -> Vec<PseudonymKey> {
    (1..=4).map(pk).collect()
}

impl World {
    /// The height-2 body over the cut `head`.
    fn at(&self, head: &VerifiedOp) -> Checkpoint {
        body(2, Some(&self.cp1), &[head], &members())
    }

    /// The two height-2 bodies over the cuts `x` and `y`, the higher
    /// digest first: a round-1 hint given the lower digest shows the round
    /// is compared before the digest.
    fn by_digest(&self) -> (Checkpoint, Checkpoint) {
        let (a, b) = (self.at(&self.x), self.at(&self.y));
        if checkpoint_digest(&a) > checkpoint_digest(&b) {
            (a, b)
        } else {
            (b, a)
        }
    }

    /// `author`'s endorsement of `c` in `round`, at seq `tag` (unique).
    fn vote(&self, author: u8, tag: u64, c: &Checkpoint, round: u32) -> VerifiedOp {
        let head = if c.cut[0].op_hash == self.x.hash() {
            &self.x
        } else {
            &self.y
        };
        let c = Checkpoint { round, ..c.clone() };
        op(
            &pk(author),
            tag,
            &[&self.cp1, head],
            tag,
            GovernanceEntry::Checkpoint(c),
        )
    }

    fn merged(&self, votes: &[VerifiedOp]) -> MergeOutcome {
        let mut ops = self.ops.clone();
        ops.extend_from_slice(votes);
        merge_outcome(&cert(&pk(1), AdmissionMode::Open), &ops)
    }

    fn with(&self, votes: &[VerifiedOp]) -> Vec<VerifiedOp> {
        let mut ops = self.ops.clone();
        ops.extend_from_slice(votes);
        ops
    }
}

#[test]
fn a_checkpoint_with_a_strict_majority_is_a_usable_hint() {
    let w = world();
    let a = w.at(&w.x);
    let two = [w.vote(1, 200, &a, 0), w.vote(2, 201, &a, 0)];
    assert!(w.merged(&two).hints.selected(2).is_none(), "2 of 4");
    let three = [two[0].clone(), two[1].clone(), w.vote(3, 202, &a, 0)];
    let outcome = w.merged(&three);
    let hint = outcome.hints.selected(2).expect("3 of 4 is usable");
    assert_eq!((hint.digest, hint.round), (checkpoint_digest(&a), 0));
    assert_eq!(hint.electorate, 4);
    let first = outcome
        .hints
        .selected(1)
        .expect("the creator alone at height 1");
    assert_eq!(first.op, w.cp1.hash());
}

#[test]
fn a_member_endorses_one_digest_per_height_and_round() {
    let w = world();
    let a = w.at(&w.x);
    let proposal = w.vote(1, 200, &a, 0);
    let ops = w.with(std::slice::from_ref(&proposal));
    let outcome = w.merged(std::slice::from_ref(&proposal));
    let mine = endorsement(&pk(2), &ops, &outcome, 2, false, |_| true).expect("round 0");
    assert_eq!(
        (checkpoint_digest(&mine), mine.round),
        (checkpoint_digest(&a), 0)
    );
    assert!(
        endorsement(&pk(2), &ops, &outcome, 2, false, |_| false).is_none(),
        "never a body it has not recomputed"
    );
    let signed = [proposal, w.vote(2, 201, &mine, 0)];
    let (ops, outcome) = (w.with(&signed), w.merged(&signed));
    assert!(
        endorsement(&pk(2), &ops, &outcome, 2, false, |_| true).is_none(),
        "write-once per (height, round)"
    );
}

#[test]
fn two_digests_for_one_cut_fall_back_to_full_replay_and_are_shown() {
    let w = world();
    let a = w.at(&w.x);
    let lie = Checkpoint {
        state_root: [9; 32],
        state_size: 0,
        ..a.clone()
    };
    let votes = [
        w.vote(1, 200, &a, 0),
        w.vote(2, 201, &a, 0),
        w.vote(3, 202, &a, 0),
        w.vote(2, 203, &lie, 1),
        w.vote(3, 204, &lie, 1),
        w.vote(4, 205, &lie, 1),
    ];
    let outcome = w.merged(&votes);
    assert_eq!(
        outcome
            .hints
            .usable
            .iter()
            .filter(|h| h.height == 2)
            .count(),
        2
    );
    assert!(outcome.hints.selected(2).is_none() && outcome.hints.selected(1).is_none());
    let equivocation = &outcome.hints.equivocations[0];
    assert_eq!(equivocation.flagged, vec![pk(2), pk(3)]);
    let digest = |o: &VerifiedOp| match o.body() {
        GovernanceEntry::Checkpoint(c) => checkpoint_digest(c),
        _ => unreachable!("a vote"),
    };
    for v in &votes {
        let cause = outcome.quarantined.iter().find_map(|(h, r)| match r {
            QuarantineReason::CheckpointEquivocation { cause } if *h == v.hash() => Some(*cause),
            _ => None,
        });
        let cause = votes
            .iter()
            .filter(|o| digest(o) != digest(v))
            .min_by_key(|o| (o.lamport(), o.hash()))
            .filter(|o| Some(o.hash()) == cause);
        assert!(cause.is_some(), "the lowest op of the other digest");
        assert!(!outcome.applied.contains(&v.hash()));
    }
}

#[test]
fn a_split_vote_opens_round_one_and_is_usable() {
    let w = world();
    let (a, b) = (w.at(&w.x), w.at(&w.y));
    let split = [
        w.vote(1, 200, &a, 0),
        w.vote(2, 201, &a, 0),
        w.vote(3, 202, &b, 0),
        w.vote(4, 203, &b, 0),
    ];
    let (ops, outcome) = (w.with(&split), w.merged(&split));
    assert!(outcome.hints.selected(2).is_none(), "2 and 2 of 4");
    assert!(endorsement(&pk(3), &ops, &outcome, 2, false, |_| true).is_none());
    let next = endorsement(&pk(3), &ops, &outcome, 2, true, |_| true).expect("round 1");
    assert_eq!(next.round, 1);
    let mut all = split.to_vec();
    all.extend([
        w.vote(1, 210, &next, 1),
        w.vote(2, 211, &next, 1),
        w.vote(3, 212, &next, 1),
    ]);
    let hint = w.merged(&all).hints.selected(2).cloned().expect("usable");
    assert_eq!((hint.round, hint.digest), (1, checkpoint_digest(&next)));
}

#[test]
fn a_timeout_never_lowers_the_majority() {
    let w = world();
    let a = w.at(&w.x);
    let votes = [
        w.vote(1, 200, &a, 0),
        w.vote(2, 201, &a, 0),
        w.vote(1, 202, &a, 1),
        w.vote(2, 203, &a, 1),
        w.vote(1, 204, &a, 2),
        w.vote(2, 205, &a, 2),
    ];
    let outcome = w.merged(&votes);
    assert!(
        outcome.hints.usable.iter().all(|h| h.height != 2),
        "2 of 4 in every round"
    );
    assert!(outcome.quarantined.is_empty());
}

#[test]
fn a_second_endorsement_in_one_round_counts_for_neither_and_is_quarantined() {
    let w = world();
    let (a, b) = (w.at(&w.x), w.at(&w.y));
    let second = w.vote(3, 203, &b, 0);
    let first = w.vote(3, 202, &a, 0);
    let votes = [
        w.vote(1, 200, &a, 0),
        w.vote(2, 201, &a, 0),
        first.clone(),
        second.clone(),
    ];
    let outcome = w.merged(&votes);
    assert!(
        outcome.hints.selected(2).is_none(),
        "3's vote counts for neither"
    );
    for (v, other) in [(&first, &second), (&second, &first)] {
        assert!(outcome.quarantined.contains(&(
            v.hash(),
            QuarantineReason::TwoCheckpointEndorsementsInOneRound {
                cause: other.hash()
            }
        )));
    }
}

#[test]
fn the_lowest_round_with_a_majority_is_selected_ties_by_digest() {
    let mut w = world();
    let c1 = w.ops[1].clone();
    // A second height-1 hint over an earlier cut, where the creator alone
    // is a member: its height-2 electorate is the creator alone.
    let early = op(
        &pk(1),
        7,
        &[&c1, &w.cp1],
        103,
        GovernanceEntry::Checkpoint(Checkpoint {
            round: 1,
            ..body(1, None, &[&c1], &[pk(1)])
        }),
    );
    w.ops.push(early.clone());
    let solo = body(2, Some(&early), &[&w.x], &[pk(1)]);
    let a = w.at(&w.y);
    let late = w.at(&w.x);
    let votes = [
        op(
            &pk(1),
            200,
            &[&early, &w.x],
            200,
            GovernanceEntry::Checkpoint(solo.clone()),
        ),
        w.vote(2, 201, &a, 0),
        w.vote(3, 202, &a, 0),
        w.vote(4, 203, &a, 0),
        w.vote(2, 204, &late, 1),
        w.vote(3, 205, &late, 1),
        w.vote(4, 206, &late, 1),
    ];
    let hints = w.merged(&votes).hints;
    assert_eq!(hints.usable.iter().filter(|h| h.height == 2).count(), 3);
    let lowest = checkpoint_digest(&solo).min(checkpoint_digest(&a));
    let hint = hints.selected(2).expect("a hint");
    assert_eq!((hint.round, hint.digest), (0, lowest));
}

#[test]
fn a_late_round_zero_majority_becomes_the_hint_and_undoes_nothing() {
    let w = world();
    let (a, b) = w.by_digest();
    let votes = vec![
        w.vote(1, 200, &a, 0),
        w.vote(2, 201, &a, 0),
        w.vote(1, 202, &b, 1),
        w.vote(2, 203, &b, 1),
        w.vote(3, 204, &b, 1),
    ];
    let before = w.merged(&votes);
    let hint = before.hints.selected(2).expect("round 1");
    assert_eq!((hint.round, hint.digest), (1, checkpoint_digest(&b)));
    let late = w.vote(4, 205, &a, 0);
    let mut all = votes.clone();
    all.push(late.clone());
    let after = w.merged(&all);
    let hint = after.hints.selected(2).expect("round 0 now");
    assert_eq!((hint.round, hint.digest), (0, checkpoint_digest(&a)));
    assert_eq!(after.state, before.state);
    let mut applied = before.applied.clone();
    applied.push(late.hash());
    applied.sort_unstable();
    let mut now = after.applied.clone();
    now.sort_unstable();
    assert_eq!(now, applied, "nothing undone");
    assert_eq!(after.quarantined, before.quarantined);
}

#[test]
fn two_cuts_at_one_height_are_both_usable_hints() {
    let w = world();
    let (a, b) = w.by_digest();
    let votes = [
        w.vote(1, 200, &a, 0),
        w.vote(2, 201, &a, 0),
        w.vote(3, 202, &a, 0),
        w.vote(2, 203, &b, 1),
        w.vote(3, 204, &b, 1),
        w.vote(4, 205, &b, 1),
    ];
    let outcome = w.merged(&votes);
    assert!(outcome.hints.equivocations.is_empty());
    assert!(outcome.quarantined.is_empty());
    assert!(outcome.hints.is_usable(2, 0, &checkpoint_digest(&a)));
    assert!(outcome.hints.is_usable(2, 1, &checkpoint_digest(&b)));
    assert_eq!(outcome.hints.selected(2).map(|h| h.round), Some(0));
}

#[test]
fn a_member_writing_outside_every_cut_never_blocks_a_checkpoint() {
    let mut w = world();
    // Member 4 keeps writing on its own join, outside every cut.
    let mut last = w.ops[4].clone();
    for seq in 1..=3 {
        let next = op(&pk(4), seq, &[&last], 300 + seq, meta("outside"));
        w.ops.push(next.clone());
        last = next;
    }
    let a = w.at(&w.x);
    let votes = [
        w.vote(1, 200, &a, 0),
        w.vote(2, 201, &a, 0),
        w.vote(3, 202, &a, 0),
    ];
    let (ops, outcome) = (w.with(&votes), w.merged(&votes));
    assert!(outcome.hints.selected(2).is_some());
    assert!(endorsement(&pk(4), &ops, &outcome, 2, false, |_| true).is_some());
}

/// A creator-only community whose record keys parse, so the directory
/// builds: the body recomputes, and a changed body does not.
#[test]
fn an_endorser_recomputes_the_identical_body_before_signing() {
    let key = format!("VLD0:{}", "A".repeat(43));
    let mut c = cert(&pk(1), AdmissionMode::Open);
    c.creator_member_record = format!("{key}:{}", "A".repeat(43));
    c.creator_member_record_owner = key;
    let c0 = op(&pk(1), 0, &[], 0, meta("one"));
    let c1 = op(&pk(1), 1, &[&c0], 1, meta("two"));
    let ops = vec![c0.clone(), c1.clone()];
    let replay = Replay {
        cert: &c,
        ops: &ops,
        community_id: [3; 32],
    };
    let built = replay
        .body(None, &[c1.op_ref("VLD0:fixture")])
        .expect("body");
    assert_eq!((built.height, built.dir_size), (1, 1));
    assert!(replay.recomputes(&built));
    let changed = Checkpoint {
        mek_generation: built.mek_generation + 1,
        ..built.clone()
    };
    assert!(!replay.recomputes(&changed));
    let unheld = op(&pk(1), 2, &[&c1], 2, meta("three"));
    assert!(replay
        .body(None, &[unheld.op_ref("VLD0:fixture")])
        .is_none());
}
