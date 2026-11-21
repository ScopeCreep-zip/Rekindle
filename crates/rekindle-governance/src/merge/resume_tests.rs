//! `merge_from` (plan E3.2 S7c.4a): from a checkpoint's snapshot it reaches
//! exactly what `merge_outcome` reaches over the whole op set, or answers
//! `NotOnTop`.

use std::collections::{BTreeSet, HashMap, HashSet};

use proptest::prelude::*;
use rekindle_types::governance::{AdmissionMode, Checkpoint, GenesisCert, GovernanceEntry, OpHash};
use rekindle_types::id::RoleId;

use super::op_proptests::{assign, community, find, head, joined, kick, pk};
use super::{merge_cut, merge_from, merge_outcome, MergeOutcome, NotOnTop};
use crate::checkpoint::{causal_past, frontier_hash, state_preimage, state_root_of, CutSnapshot};
use crate::fixture::{cert, meta, op};
use crate::op::VerifiedOp;
use crate::power::{class_of, OpClass};

/// An op whose `issued_at` is `back` below its lamport.
fn mk(
    a: u8,
    seq: u64,
    deps: &[&VerifiedOp],
    tag: u64,
    e: GovernanceEntry,
    back: u64,
) -> VerifiedOp {
    let base = op(&pk(a), seq, deps, tag, e);
    let mut body = base.op().clone();
    body.issued_at = body.issued_at.saturating_sub(back);
    VerifiedOp::assume_verified(body, base.hash(), base.lamport())
}

/// A checkpoint body over `heads` whose `stateRoot` is that of `bytes`.
fn body(heads: &[&VerifiedOp], bytes: &[u8]) -> Checkpoint {
    let hashes: Vec<OpHash> = heads.iter().map(|o| o.hash()).collect();
    Checkpoint {
        height: 1,
        round: 0,
        prev: None,
        cut: heads.iter().map(|o| o.op_ref("VLD0:fixture")).collect(),
        frontier_hash: frontier_hash(&hashes),
        dir_root: [0; 32],
        dir_size: 0,
        state_root: state_root_of(bytes),
        state_size: u32::try_from(bytes.len()).expect("a test snapshot fits"),
        mek_generation: 1,
        eligible_set_hash: [0; 32],
        pages: Vec::new(),
    }
}

/// The snapshot at the cut whose heads are `heads` (the maximal ops of
/// their closure), the cut's power ops, and every op of `all` outside it.
fn split(
    cert: &GenesisCert,
    all: &[VerifiedOp],
    heads: &[&VerifiedOp],
) -> (CutSnapshot, Vec<VerifiedOp>, Vec<VerifiedOp>) {
    let refs: Vec<_> = heads.iter().map(|o| o.op_ref("VLD0:fixture")).collect();
    let cut_ops = causal_past(all, &refs);
    let (state, cut) = merge_cut(cert, &cut_ops);
    let bytes = state_preimage(&state, &cut);
    let snap = CutSnapshot::decode(&body(heads, &bytes), &bytes).expect("a canonical snapshot");
    assert_eq!(
        (&snap.state, &snap.cut),
        (&state, &cut),
        "the snapshot decodes to itself"
    );
    let in_cut: HashSet<OpHash> = cut_ops.iter().map(VerifiedOp::hash).collect();
    let power = located_power(cert, &snap, all);
    let rest = all
        .iter()
        .filter(|o| !in_cut.contains(&o.hash()))
        .cloned()
        .collect();
    (snap, power, rest)
}

/// The cut's applied power ops `snap` lists, each taken from `all` by its
/// locator alone (author, seq), as a reader holding only the snapshot
/// fetches them, then its refused ones (those a post-cut op cites are
/// fetched by its locator). With `power_refs` `None` a reader replays in
/// full; they are then taken by hash so `merge_from` is still checked.
fn located_power(cert: &GenesisCert, snap: &CutSnapshot, all: &[VerifiedOp]) -> Vec<VerifiedOp> {
    let fate = |o: &&VerifiedOp, a: bool| {
        (snap.cut.power.iter()).any(|(c, f)| *f == a && c.hash == o.hash())
    };
    let refused = all.iter().filter(|o| fate(o, false)).cloned();
    let Some(refs) = snap.power_refs(cert) else {
        return all
            .iter()
            .filter(|o| fate(o, true))
            .cloned()
            .chain(refused)
            .collect();
    };
    (refs.iter())
        .map(|r| {
            let at = |o: &&VerifiedOp| *o.author() == r.author && o.op().seq == r.author_seq;
            let op = all.iter().filter(at).find(|o| o.hash() == r.op_hash);
            op.cloned().expect("the op at the located author and seq")
        })
        .chain(refused)
        .collect()
}

/// `merge_from` over the split at `heads`, checked against `merge_outcome`
/// whenever it answers `Ok`: the state, the applied order, every post-cut
/// fate and the cut's power fates.
pub(super) fn resume(
    cert: &GenesisCert,
    all: &[VerifiedOp],
    heads: &[&VerifiedOp],
) -> Result<MergeOutcome, NotOnTop> {
    let (snap, power, rest) = split(cert, all, heads);
    let resumed = merge_from(cert, &snap, &power, &rest)?;
    let snap = &snap;
    let full = merge_outcome(cert, all);
    let ours: HashSet<OpHash> = rest.iter().chain(&power).map(VerifiedOp::hash).collect();
    let post: HashSet<OpHash> = rest.iter().map(VerifiedOp::hash).collect();
    let keep = |v: &[OpHash], s: &HashSet<OpHash>| -> Vec<OpHash> {
        v.iter().filter(|h| s.contains(h)).copied().collect()
    };
    assert_eq!(resumed.state, full.state, "state");
    assert_eq!(resumed.applied, keep(&full.applied, &ours), "applied order");
    assert_eq!(
        keep(&resumed.refused, &post),
        keep(&full.refused, &post),
        "post-cut refusals"
    );
    let q = |o: &MergeOutcome| -> Vec<_> {
        (o.quarantined.iter())
            .filter(|(h, _)| post.contains(h))
            .cloned()
            .collect()
    };
    assert_eq!(q(&resumed), q(&full), "post-cut quarantines");
    for (p, _) in &snap.cut.power {
        let applied = full.applied.contains(&p.hash);
        assert_eq!(resumed.applied.contains(&p.hash), applied, "cut power fate");
        assert_eq!(
            resumed.refused.contains(&p.hash),
            !applied,
            "cut power fate"
        );
    }
    assert_eq!(
        resumed.pending,
        keep(&full.pending, &post),
        "post-cut pending"
    );
    assert_eq!(resumed.equivocators, full.equivocators, "equivocators");
    Ok(resumed)
}

/// A seeded random DAG over `community()`: authors 1–7 (5–7 join by the
/// invite first), each op citing its author's previous op and random
/// earlier ones (or, with `descend`, every cut head), with kicks carrying
/// honest or stale frontiers, role changes, approvals, leaves, revokes,
/// forks and back-dated `issued_at`.
struct Dag {
    ops: Vec<VerifiedOp>,
    past: Vec<BTreeSet<usize>>,
    last: HashMap<u8, usize>,
    x: u64,
}

impl Dag {
    fn new(seed: u64) -> Self {
        let ops = community();
        let mut past: Vec<BTreeSet<usize>> = Vec::new();
        for o in &ops {
            let mut p = BTreeSet::new();
            for d in &o.op().deps {
                let i = ops.iter().position(|x| x.hash() == d.op_hash).expect("dep");
                p.insert(i);
                p.extend(past[i].iter().copied());
            }
            past.push(p);
        }
        let last = (1..=4u8)
            .map(|a| {
                (
                    a,
                    (0..ops.len())
                        .rev()
                        .find(|&i| ops[i].author() == &pk(a))
                        .expect("op"),
                )
            })
            .collect();
        Self {
            ops,
            past,
            last,
            x: seed | 1,
        }
    }

    fn below(&mut self, n: usize) -> usize {
        self.x ^= self.x << 13;
        self.x ^= self.x >> 7;
        self.x ^= self.x << 17;
        usize::try_from(self.x % n.max(1) as u64).unwrap_or(0)
    }

    fn push(&mut self, cite: &[usize], forks: bool, back_dates: bool) {
        let a = u8::try_from(1 + self.below(7)).unwrap_or(1);
        let mut deps: BTreeSet<usize> = cite.iter().copied().collect();
        let fork = forks && self.below(12) == 0;
        let (seq, prev) = match self.last.get(&a).copied() {
            Some(l) if fork => (self.ops[l].op().seq, self.ops[l].op().prev),
            Some(l) => (self.ops[l].op().seq + 1, Some(self.ops[l].hash())),
            None => (0, None),
        };
        if let Some(i) = prev.and_then(|h| self.ops.iter().position(|o| o.hash() == h)) {
            deps.insert(i);
        }
        let mut past: BTreeSet<usize> = BTreeSet::new();
        for &d in &deps {
            past.insert(d);
            past.extend(self.past[d].iter().copied());
        }
        let t = u8::try_from(2 + self.below(6)).unwrap_or(2);
        let (kind, stale, role) = (self.below(8), self.below(4) == 0, self.below(2));
        let count = past
            .iter()
            .filter(|&&i| self.ops[i].author() == &pk(t))
            .count() as u64;
        // Without forks, removals and role changes name members only (a
        // removal before its target joined is undecidable, see
        // `a_rejoin_after_a_cut_removal_is_undecidable`).
        let entry = if a >= 5 && seq == 0 {
            joined(a, Some("code"))
        } else if !forks && count == 0 && (2..=5).contains(&kind) {
            meta("m")
        } else {
            match kind {
                0 | 1 => meta("m"),
                2 => kick(t, count.saturating_sub(u64::from(stale))),
                3 => assign(t, if role == 0 { 5 } else { 6 }),
                4 => GovernanceEntry::RoleUnassignment {
                    target: pk(t),
                    role_id: RoleId([6; 16]),
                },
                5 => GovernanceEntry::MemberApproved { target: pk(t) },
                6 => GovernanceEntry::MemberLeft { heads: Vec::new() },
                _ => GovernanceEntry::InviteRevoked { invite_id: [7; 16] },
            }
        };
        let back = if back_dates && self.below(6) == 0 {
            self.below(8) as u64
        } else {
            0
        };
        let tag = 1000 + self.ops.len() as u64;
        let refs: Vec<&VerifiedOp> = deps.iter().map(|&d| &self.ops[d]).collect();
        let next = mk(a, seq, &refs, tag, entry, back);
        self.last.insert(a, self.ops.len());
        self.past.push(past);
        self.ops.push(next);
    }

    /// Random earlier ops to cite.
    fn some(&mut self) -> Vec<usize> {
        let n = self.ops.len();
        (0..=self.below(3))
            .map(|_| n - 1 - self.below(n.min(6)))
            .collect()
    }

    /// The maximal ops among the closure of `picked`.
    fn heads(&self, picked: &[usize]) -> Vec<usize> {
        let closure: BTreeSet<usize> = picked
            .iter()
            .flat_map(|&p| self.past[p].iter().copied().chain([p]))
            .collect();
        closure
            .iter()
            .copied()
            .filter(|&i| !closure.iter().any(|&j| self.past[j].contains(&i)))
            .collect()
    }
}

fn mode(pick: usize) -> GenesisCert {
    let policy = if pick.is_multiple_of(2) {
        AdmissionMode::Open
    } else {
        AdmissionMode::ApprovalRequired
    };
    cert(&pk(1), policy)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(160))]

    /// Random op sets and random cuts (random head subsets, so concurrent
    /// ops, power ops, forks and back-dated `issued_at` arise): whenever
    /// `merge_from` answers `Ok` it equals `merge_outcome`, and it answers
    /// `Ok` whenever every post-cut op descends from the whole cut, sits
    /// above its author's cut seq, (non-power) is dated above the cut, and
    /// no gate is undecidable.
    #[test]
    fn merge_from_equals_merge_outcome_whenever_it_answers_ok(seed in any::<u64>()) {
        let mut dag = Dag::new(seed);
        for _ in 0..4 + dag.below(10) {
            let cite = dag.some();
            dag.push(&cite, true, true);
        }
        let pre = dag.ops.len();
        let picked: Vec<usize> = if dag.below(2) == 0 { (0..pre).collect() } else { (0..=dag.below(4)).map(|_| dag.below(pre)).collect() };
        let heads = dag.heads(&picked);
        for _ in 0..3 + dag.below(10) {
            let mut cite = dag.some();
            if dag.below(4) != 0 {
                cite.extend(&heads);
            }
            dag.push(&cite, true, true);
        }
        let cert = mode(dag.below(2));
        let head_ops: Vec<&VerifiedOp> = heads.iter().map(|&h| &dag.ops[h]).collect();
        let answer = resume(&cert, &dag.ops, &head_ops);
        let cut: BTreeSet<usize> = heads.iter().flat_map(|&h| dag.past[h].iter().copied().chain([h])).collect();
        let refs: Vec<_> = head_ops.iter().map(|o| o.op_ref("VLD0:fixture")).collect();
        let cut_index = merge_cut(&cert, &causal_past(&dag.ops, &refs)).1;
        let on_top = (0..dag.ops.len()).filter(|i| !cut.contains(i)).all(|i| {
            let o = &dag.ops[i];
            let whole = cut.iter().all(|c| dag.past[i].contains(c));
            let top = cut_index.authors.get(&o.author().0).map(|(s, _)| *s);
            let top = top.max(cut_index.unmerged.get(&o.author().0).copied());
            let above = top.is_none_or(|s| o.op().seq > s);
            let dated = class_of(o.body()) == OpClass::Power || Some((o.op().issued_at, o.hash())) > cut_index.mainline_max;
            whole && above && dated
        });
        // Beside (a′), (c) and (d), only an undecidable gate or a fork (its
        // ops stay ancestry and the full merge re-judges what they carry,
        // ADR 0022 rule 5) answers no.
        let decided = !matches!(answer, Err(NotOnTop::Undecidable(_) | NotOnTop::Forked { .. }));
        prop_assert!(!on_top || !decided || answer.is_ok(), "on top but {:?}", answer.err());
    }

    /// `merge_from_equals_full_merge_when_power_ops_descend_from_the_cut`:
    /// approvals, kicks and role changes after the cut, each citing the
    /// whole cut, never send the reader to a full replay.
    #[test]
    fn merge_from_equals_full_merge_when_power_ops_descend_from_the_cut(seed in any::<u64>()) {
        let mut dag = Dag::new(seed);
        for _ in 0..3 + dag.below(8) {
            let cite = dag.some();
            dag.push(&cite, false, false);
        }
        let all: Vec<usize> = (0..dag.ops.len()).collect();
        let heads = dag.heads(&all);
        for _ in 0..3 + dag.below(10) {
            let mut cite = dag.some();
            cite.extend(&heads);
            dag.push(&cite, false, false);
        }
        let cert = mode(dag.below(2));
        let head_ops: Vec<&VerifiedOp> = heads.iter().map(|&h| &dag.ops[h]).collect();
        let answer = resume(&cert, &dag.ops, &head_ops);
        prop_assert!(answer.is_ok(), "{:?}", answer.err());
    }
}

/// `community()` with its last op as the only head.
fn base() -> (GenesisCert, Vec<VerifiedOp>) {
    (cert(&pk(1), AdmissionMode::Open), community())
}

/// S7c.4a's (b) is implied by (a′): a kick citing the whole cut but
/// claiming a frontier below its target's cut ops fails `frontier_holds`,
/// is refused, masks nothing, and the merge stays on top.
#[test]
fn a_kick_frontier_below_the_cut_is_refused_and_stays_on_top() {
    let (cert, mut ops) = base();
    let m = op(&pk(4), 1, &[head(&ops), find(&ops, 4, 0)], 40, meta("x"));
    ops.push(m);
    let k = op(&pk(2), 1, &[head(&ops), find(&ops, 2, 0)], 41, kick(4, 1));
    let hash = k.hash();
    ops.push(k);
    let heads = [&ops[ops.len() - 2]];
    let resumed = resume(&cert, &ops, &heads).expect("on top");
    assert!(resumed.refused.contains(&hash));
}

#[test]
fn a_fork_into_the_cut_is_not_on_top() {
    let (cert, ops) = base();
    // 4 signs a second op at seq 0, outside the cut.
    let fork = op(&pk(4), 0, &[head(&ops)], 50, meta("fork"));
    let hash = fork.hash();
    let mut all = ops.clone();
    all.push(fork);
    let heads = [&all[ops.len() - 1]];
    assert_eq!(
        resume(&cert, &all, &heads).err(),
        Some(NotOnTop::IntoTheCut(hash))
    );
}

#[test]
fn a_back_dated_op_is_not_on_top() {
    let (cert, mut ops) = base();
    let late = mk(4, 1, &[head(&ops), find(&ops, 4, 0)], 50, meta("late"), 9);
    let hash = late.hash();
    ops.push(late);
    let heads = [&ops[ops.len() - 2]];
    assert_eq!(
        resume(&cert, &ops, &heads).err(),
        Some(NotOnTop::BelowTheCut(hash))
    );
}

/// A `Checkpoint` op changes no state, but its fate is its author's
/// membership at its mainline position (`validate::permitted`), so a
/// back-dated one is held to (d) like any non-power op.
#[test]
fn a_checkpoint_op_with_an_early_issued_at_is_not_on_top() {
    let (cert, ops) = base();
    let heads = [&ops[ops.len() - 1]];
    let checkpoint = GovernanceEntry::Checkpoint(body(&heads, &[]));
    let early = mk(
        2,
        1,
        &[head(&ops), find(&ops, 2, 0)],
        60,
        checkpoint.clone(),
        9,
    );
    let on_time = mk(3, 1, &[head(&ops), find(&ops, 3, 0)], 61, checkpoint, 0);
    let mut all = ops.clone();
    all.push(on_time);
    resume(&cert, &all, &[&all[ops.len() - 1]]).expect("an on-time checkpoint op is on top");
    let hash = early.hash();
    all.push(early);
    let answer = resume(&cert, &all, &[&all[ops.len() - 1]]);
    assert_eq!(answer.err(), Some(NotOnTop::BelowTheCut(hash)));
}

#[test]
fn invite_uses_are_in_the_state_and_survive_a_resume() {
    let c0 = op(&pk(1), 0, &[], 0, joined(1, None));
    let c1 = op(&pk(1), 1, &[&c0], 1, super::op_proptests::invite(3));
    let j2 = op(&pk(2), 0, &[&c1], 2, joined(2, Some("code")));
    let j3 = op(&pk(3), 0, &[&j2], 3, joined(3, Some("code")));
    let j4 = op(&pk(4), 0, &[&j3], 4, joined(4, Some("code")));
    let j5 = op(&pk(5), 0, &[&j4], 5, joined(5, Some("code")));
    let all = vec![c0, c1, j2, j3, j4, j5];
    let cert = cert(&pk(1), AdmissionMode::Open);
    let resumed = resume(&cert, &all, &[&all[3]]).expect("on top");
    assert_eq!(
        resumed.state.invites[&[7; 16]].uses, 3,
        "two before the cut, one after"
    );
    assert!(
        resumed.refused.contains(&all[5].hash()),
        "the fourth join is past the cap"
    );
}

#[test]
fn a_snapshot_that_is_not_canonical_is_refused() {
    let (cert, ops) = base();
    let (state, cut) = merge_cut(&cert, &ops);
    let mut bytes = state_preimage(&state, &cut);
    let heads = [&ops[ops.len() - 1]];
    assert!(CutSnapshot::decode(&body(&heads, &bytes), &bytes).is_some());
    bytes.push(0);
    assert!(
        CutSnapshot::decode(&body(&heads, &bytes), &bytes).is_none(),
        "a trailing byte"
    );
    let short = &bytes[..bytes.len() - 2];
    assert!(
        CutSnapshot::decode(&body(&heads, short), short).is_none(),
        "a cut short snapshot"
    );
}

/// The snapshot grows with members and power ops, never with the history
/// of a value one op overwrites (plan E3.2 S7c.3: O(N + power ops)).
#[test]
fn snapshot_size_is_linear_in_members_and_power_ops() {
    let size = |members: u8, kicks: u8, metas: u64| -> usize {
        let c0 = op(&pk(1), 0, &[], 0, joined(1, None));
        let c1 = op(&pk(1), 1, &[&c0], 1, super::op_proptests::invite(100));
        let mut ops = vec![c0, c1];
        for m in 0..members {
            let j = op(
                &pk(10 + m),
                0,
                &[head(&ops)],
                10 + u64::from(m),
                joined(10 + m, Some("code")),
            );
            ops.push(j);
        }
        let mut seq = 2;
        for k in 0..kicks {
            let kick = op(
                &pk(1),
                seq,
                &[head(&ops)],
                200 + u64::from(k),
                kick(10 + k, 1),
            );
            ops.push(kick);
            seq += 1;
        }
        for t in 0..metas {
            let next = op(&pk(1), seq, &[head(&ops)], 400 + t, meta("m"));
            ops.push(next);
            seq += 1;
        }
        let (state, cut) = merge_cut(&cert(&pk(1), AdmissionMode::Open), &ops);
        state_preimage(&state, &cut).len()
    };
    assert_eq!(
        size(4, 0, 1),
        size(4, 0, 50),
        "history of one value adds nothing"
    );
    let per_member = size(5, 0, 1) - size(4, 0, 1);
    assert_eq!(
        size(9, 0, 1) - size(8, 0, 1),
        per_member,
        "linear in members"
    );
    let per_kick = size(6, 2, 1) - size(6, 1, 1);
    assert_eq!(
        size(6, 4, 1) - size(6, 3, 1),
        per_kick,
        "linear in power ops"
    );
}

#[path = "resume_gate_tests.rs"]
mod gate_tests;

/// P3: a reader holding only the snapshot fetches the cut power ops by locator.
#[test]
fn a_cut_power_op_is_located_through_the_directory() {
    let (cert, mut ops) = base();
    let m = op(&pk(4), 1, &[head(&ops), find(&ops, 4, 0)], 40, meta("x"));
    ops.push(m);
    // Manager 2's kick of 4: a member's applied power op.
    let k = op(&pk(2), 1, &[head(&ops), find(&ops, 2, 0)], 41, kick(4, 2));
    ops.push(k);
    let heads = [head(&ops)];
    let (snap, power, rest) = split(&cert, &ops, &heads);
    assert!(snap.power_refs(&cert).is_some(), "all located");
    let listed: Vec<OpHash> = (snap.cut.power.iter())
        .filter(|(_, applied)| *applied)
        .map(|(c, _)| c.hash)
        .collect();
    assert!(listed.len() > 1, "the community has power ops");
    assert_eq!(
        power.iter().map(VerifiedOp::hash).collect::<Vec<_>>(),
        listed
    );
    assert!(power.iter().any(|o| *o.author() != cert.creator_pseudonym));
    assert!(merge_from(&cert, &snap, &power, &rest).is_ok());
}
