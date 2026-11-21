//! The resumable merge (plan E3.2 S7c.4a): `merge_from` continues
//! `merge_outcome` from a checkpoint's state snapshot instead of replaying
//! the cut's ops, and answers [`NotOnTop`] unless it reaches exactly what
//! the full merge would (an exact precondition, never an approximate
//! continuation, so a hint never changes a verdict).
//!
//! `rest` is **on top** of the cut when (a′) every op in it has every cut
//! head in its causal past (a mask over the at most 63 heads, computed
//! from deps alone, since no cut op has a head in its past) or is beside
//! the cut as below, (c) no op in
//! it sits at or below its author's highest seq in the cut, and (d) every
//! non-power op in it has a mainline key above every cut non-power op's.
//! Under (a′) every cut op is in every `rest` op's causal past, so each
//! question the gates ask between a `rest` op and a cut op has the answer
//! "in its past", the cut's power order is a prefix of the full one, each
//! `rest` op's anchor is at or after the cut's last applied power op, and
//! the cut index carries what the gates read from the cut (strong-removal
//! masks, record holders, invite codes, `uses`). Under (a) a removal in
//! `rest` that applies counts every cut op of its target in its frontier
//! (`frontier_holds`), so it masks no cut op: S7c.4a's (b) holds.
//!
//! **Beside the cut** (plan E3.2 S7c.8 (3)). An op lacking a cut head (a
//! join signed concurrently with the checkpoint cites the old base) is on
//! top when it is non-power and not a `Checkpoint` op and every applied
//! cut power op is in its causal past, proven from its deps' locators (the
//! author chain: seq n cites n − 1, so an author's op at seq s has that
//! author's ops at seq ≤ s in its past) and the fetched cut power ops'
//! own deps. Then its anchor is the cut's last applied power op (no `rest`
//! power op is in its past, as each has the whole cut in its own), so with
//! (d) it sorts after every cut non-power op and is judged at the
//! snapshot's state, as an op on top is: the snapshot answers for it as
//! Matrix's partial-state join authorises an event against the state plus
//! the event's own auth events (MSC3902). What the full merge would decide
//! between it and a concurrent cut op is left to `resume_gates`, which
//! answers `NotOnTop` where order or causality could matter (a spent or
//! revoked invite, an invite not in its past, a cut member record).

use std::collections::{BTreeMap, HashMap};

use rekindle_types::governance::{GenesisCert, GovernanceEntry, OpHash};
use rekindle_types::id::PseudonymKey;

use super::resolve::Ancestry;
use super::{fold, genesis_state, try_apply, MergeOutcome, QuarantineReason};
use crate::checkpoint::{CutSnapshot, Hints};
use crate::op::VerifiedOp;
use crate::power::{class_of, removal_target, target_seq_frontier, OpClass};
use crate::state::GovernanceState;

/// Why `merge_from` cannot continue from the snapshot; the caller tries
/// the next older usable hint, then the full merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotOnTop {
    /// A cut power op the snapshot lists as applied was not supplied.
    MissingCutPowerOp(OpHash),
    /// (a′): this op lacks a cut head in its causal past, and the snapshot
    /// cannot answer for it beside the cut (S7c.8).
    BesideTheCut(OpHash),
    /// (c): this op sits at or below its author's highest seq in the cut.
    IntoTheCut(OpHash),
    /// (d): this non-power op's mainline key is not above the cut's.
    BelowTheCut(OpHash),
    /// A gate needs a fact about this op's cut deps that the snapshot does
    /// not carry (the `issued_at` of a non-power cut dep below the cut's
    /// greatest, a previous checkpoint in the cut that the cut index does
    /// not list among its applied checkpoints).
    Undecidable(OpHash),
    /// `author` forked at `seq`, in the cut or above it: its ops from there
    /// on stay ancestry and every op with one in its causal past is judged
    /// by the counted and final rounds (ADR 0022 rule 5, `ancestry`), over
    /// the whole op set, which the snapshot does not carry.
    /// `author` is the pseudonym's bytes (the error stays `Copy`).
    Forked { author: [u8; 32], seq: u64 },
}

/// Where an op's dep is.
#[derive(Clone, Copy)]
pub(super) enum Dep {
    Rest(usize),
    Cut,
}

/// `rest`, prepared like `resolve::prepare` but resolving a dep outside
/// `rest` into the cut when its seq is at or below its author's cut entry
/// (the merged cut ops; a dep on a cut op the merge leaves out waits, as
/// it does in the full merge).
pub(super) struct RestSet<'a> {
    pub ops: Vec<&'a VerifiedOp>,
    pub past: Ancestry,
    pub deps: Vec<Vec<Dep>>,
    pub index: HashMap<OpHash, usize>,
    pub pending: Vec<OpHash>,
    pub equivocators: Vec<(PseudonymKey, u64)>,
    /// Per op beside the cut, each author's highest cut seq its deps (and
    /// its `rest` ancestors' deps) cite; `None` for an op with every cut
    /// head in its past.
    pub reach: Vec<Option<Reach>>,
}

/// An author's highest seq cited, keyed by pseudonym bytes.
pub(super) type Reach = BTreeMap<[u8; 32], u64>;

/// Whether `reach` has the cut op at `author`'s `seq` in its past.
pub(super) fn reaches(reach: &Reach, author: &[u8; 32], seq: u64) -> bool {
    reach.get(author).is_some_and(|s| *s >= seq)
}

fn cite(reach: &mut Reach, author: [u8; 32], seq: u64) {
    let slot = reach.entry(author).or_insert(seq);
    *slot = (*slot).max(seq);
}

/// What the resume reads besides `rest`.
pub(super) struct Ctx<'a> {
    pub cert: &'a GenesisCert,
    pub snap: &'a CutSnapshot,
    /// The cut's power ops by hash, every fate.
    pub cut_power: HashMap<OpHash, &'a VerifiedOp>,
    /// The cut's applied power ops, in order.
    pub cut_applied: Vec<&'a VerifiedOp>,
    /// The power state after them.
    pub power_state: GovernanceState,
}

/// One resolution pass over `rest`.
pub(super) struct RestRound {
    pub state: GovernanceState,
    pub applied: Vec<bool>,
    /// Applied `rest` ops in judgement order (power ops first).
    pub order: Vec<usize>,
    /// The applied `rest` power ops, in their order.
    pub power_order: Vec<usize>,
    /// Each op's anchor in the full power order (the cut's applied power
    /// ops, then `power_order`).
    pub anchor: Vec<Option<usize>>,
}

/// The merge of the full op set, from `snapshot` (the state and cut index
/// at a checkpoint's cut), `cut_power` (the cut's applied power ops,
/// fetched by their locators, `CutSnapshot::power_refs`, and any refused
/// one a `rest` op cites; a refused one is never folded, so only its fate,
/// which the snapshot carries, and its `issued_at`, which a join citing it
/// is dated against, are read: one not supplied makes that join
/// `Undecidable`)
/// and `rest` (every op held outside the cut): the cut's power
/// fates followed by `rest`'s. Its `hints` are empty: the hint layer is a
/// function of every `Checkpoint` op held (`checkpoint::hints`).
///
/// # Errors
/// [`NotOnTop`] when `rest` is not on top of the cut.
pub fn merge_from(
    cert: &GenesisCert,
    snapshot: &CutSnapshot,
    cut_power: &[VerifiedOp],
    rest: &[VerifiedOp],
) -> Result<MergeOutcome, NotOnTop> {
    let ctx = context(cert, snapshot, cut_power)?;
    let set = prepare_rest(snapshot, rest)?;
    beside_anchored(&ctx, &set)?;
    let n = set.ops.len();
    let mut invalid = vec![false; n];
    let mut round = rest_round(&ctx, &set, &invalid);
    let mut passes = 0;
    let gated = loop {
        let gated = super::resume_gates::invalidated(&ctx, &set, &round)?;
        passes += 1;
        if gated.invalid == invalid || passes > n + 1 {
            break gated;
        }
        invalid = gated.invalid;
        round = rest_round(&ctx, &set, &invalid);
    };
    above_the_cut(&ctx, &set, &round)?;
    for (id, invite) in &mut round.state.invites {
        invite.uses = gated.uses.get(id).copied().unwrap_or(0);
    }
    let mut applied: Vec<OpHash> = ctx.cut_applied.iter().map(|o| o.hash()).collect();
    applied.extend(round.order.iter().map(|&i| set.ops[i].hash()));
    let mut refused: Vec<OpHash> = (snapshot.cut.power.iter())
        .filter(|(_, fate)| !fate)
        .map(|(c, _)| c.hash)
        .collect();
    refused.extend(
        (0..n)
            .filter(|&i| !round.applied[i] && gated.masked_by[i].is_none())
            .filter(|i| !gated.record_refusals.contains_key(i))
            .map(|i| set.ops[i].hash()),
    );
    let mut quarantined: Vec<(OpHash, QuarantineReason)> = (0..n)
        .filter_map(|i| {
            let cause = gated.masked_by[i]?;
            Some((
                set.ops[i].hash(),
                QuarantineReason::AfterAuthorRemoval { cause },
            ))
        })
        .collect();
    quarantined.extend(
        (gated.record_refusals.iter())
            .filter(|&(&i, _)| !round.applied[i] && gated.masked_by[i].is_none())
            .map(|(&i, holder)| {
                (
                    set.ops[i].hash(),
                    QuarantineReason::RecordClaimed {
                        holder: holder.clone(),
                    },
                )
            }),
    );
    let join_refusals = (gated.invite_refusals.iter())
        .filter(|&(&i, _)| !round.applied[i] && gated.masked_by[i].is_none())
        .map(|(&i, &r)| (set.ops[i].hash(), r))
        .collect();
    Ok(MergeOutcome {
        state: round.state,
        applied,
        refused,
        quarantined,
        pending: set.pending,
        equivocators: set.equivocators,
        hints: Hints::default(),
        join_refusals,
    })
}

fn context<'a>(
    cert: &'a GenesisCert,
    snap: &'a CutSnapshot,
    cut_power: &'a [VerifiedOp],
) -> Result<Ctx<'a>, NotOnTop> {
    let by_hash: HashMap<OpHash, &VerifiedOp> = cut_power.iter().map(|o| (o.hash(), o)).collect();
    let mut cut_applied = Vec::new();
    let mut power_state = genesis_state(cert);
    for (at, _) in snap.cut.power.iter().filter(|(_, applied)| *applied) {
        let op = *by_hash
            .get(&at.hash)
            .ok_or(NotOnTop::MissingCutPowerOp(at.hash))?;
        fold(op, &mut power_state);
        cut_applied.push(op);
    }
    Ok(Ctx {
        cert,
        snap,
        cut_power: by_hash,
        cut_applied,
        power_state,
    })
}

/// `rest` in stamp order, deduplicated, refused (`NotOnTop::Forked`) when
/// an author forked in the cut or among `rest`'s own ops (`resolve::prepare`
/// keeps its ops from the fork on as ancestry over the whole set), each
/// op's deps
/// resolved, (c) and (a′) checked.
fn prepare_rest<'a>(snap: &CutSnapshot, rest: &'a [VerifiedOp]) -> Result<RestSet<'a>, NotOnTop> {
    let cut_seq = |author: &PseudonymKey| snap.cut.authors.get(&author.0).map(|(s, _)| *s);
    let cut_top = |author: &PseudonymKey| {
        let unmerged = snap.cut.unmerged.get(&author.0).copied();
        cut_seq(author).max(unmerged)
    };
    let mut sorted: Vec<&VerifiedOp> = rest.iter().collect();
    sorted.sort_by_key(|op| op.stamp());
    sorted.dedup_by_key(|op| op.hash());
    if let Some(op) = (sorted.iter()).find(|o| cut_top(o.author()).is_some_and(|s| o.op().seq <= s))
    {
        return Err(NotOnTop::IntoTheCut(op.hash()));
    }
    let mut by_seq: Vec<(u64, [u8; 32], OpHash)> = (sorted.iter())
        .map(|o| (o.op().seq, o.author().0, o.hash()))
        .collect();
    by_seq.sort_unstable();
    let mut fork_at: BTreeMap<[u8; 32], u64> = snap.cut.forks.clone();
    for pair in by_seq.windows(2) {
        if pair[0].1 == pair[1].1 && pair[0].0 == pair[1].0 && pair[0].2 != pair[1].2 {
            let slot = fork_at.entry(pair[0].1).or_insert(u64::MAX);
            *slot = (*slot).min(pair[0].0);
        }
    }
    if let Some((author, seq)) = fork_at.iter().next() {
        return Err(NotOnTop::Forked {
            author: *author,
            seq: *seq,
        });
    }
    let head_bit: HashMap<OpHash, u64> = (snap.heads.iter().enumerate())
        .map(|(k, h)| (*h, 1u64 << k))
        .collect();
    let whole = head_bit.values().fold(0u64, |m, b| m | b);
    let mut set = RestSet {
        ops: Vec::new(),
        past: Ancestry::with_capacity(sorted.len()),
        deps: Vec::new(),
        index: HashMap::new(),
        pending: Vec::new(),
        equivocators: (fork_at.iter())
            .map(|(author, seq)| (PseudonymKey(*author), *seq))
            .collect(),
        reach: Vec::new(),
    };
    let mut masks: Vec<u64> = Vec::new();
    for op in sorted {
        let mut deps = Vec::new();
        let mut mask = 0u64;
        let complete = op.op().deps.iter().all(|d| {
            if let Some(&i) = set.index.get(&d.op_hash) {
                deps.push(Dep::Rest(i));
                mask |= masks[i];
                true
            } else if cut_seq(&d.author).is_some_and(|s| d.author_seq <= s) {
                deps.push(Dep::Cut);
                mask |= head_bit.get(&d.op_hash).copied().unwrap_or(0);
                true
            } else {
                false
            }
        });
        if !complete {
            set.pending.push(op.hash());
            continue;
        }
        let reach = (mask != whole)
            .then(|| beside_reach(op, &deps, &set))
            .transpose()?;
        let in_rest: Vec<usize> = (deps.iter())
            .filter_map(|d| match d {
                Dep::Rest(i) => Some(*i),
                Dep::Cut => None,
            })
            .collect();
        set.index.insert(op.hash(), set.ops.len());
        set.ops.push(op);
        set.past.push(&in_rest);
        set.deps.push(deps);
        set.reach.push(reach);
        masks.push(mask);
    }
    Ok(set)
}

/// The cut seqs `op` (beside the cut, its deps `deps`) reaches: a power or
/// `Checkpoint` op beside the cut is not on top (a power op could reorder
/// the cut's power phase; a `Checkpoint`'s gate reads its cut as a
/// descendant of the whole cut).
fn beside_reach(op: &VerifiedOp, deps: &[Dep], set: &RestSet<'_>) -> Result<Reach, NotOnTop> {
    let checkpoint = matches!(op.body(), GovernanceEntry::Checkpoint(_));
    if checkpoint || class_of(op.body()) == OpClass::Power {
        return Err(NotOnTop::BesideTheCut(op.hash()));
    }
    let mut reach = Reach::new();
    for (d, at) in op.op().deps.iter().zip(deps) {
        match at {
            Dep::Cut => cite(&mut reach, d.author.0, d.author_seq),
            Dep::Rest(i) => {
                for (author, seq) in set.reach[*i].iter().flatten() {
                    cite(&mut reach, *author, *seq);
                }
            }
        }
    }
    Ok(reach)
}

/// Every op beside the cut has every applied cut power op in its past: a
/// cut power op it reaches adds its own deps (the power order is a stamp
/// order, so a power op's past comes before it and the reverse walk sees
/// each after every op that could cite it).
fn beside_anchored(ctx: &Ctx<'_>, set: &RestSet<'_>) -> Result<(), NotOnTop> {
    for (i, reach) in set.reach.iter().enumerate() {
        let Some(reach) = reach else {
            continue;
        };
        let mut reach = reach.clone();
        for q in ctx.cut_applied.iter().rev() {
            if !reaches(&reach, &q.author().0, q.op().seq) {
                return Err(NotOnTop::BesideTheCut(set.ops[i].hash()));
            }
            for d in &q.op().deps {
                cite(&mut reach, d.author.0, d.author_seq);
            }
        }
    }
    Ok(())
}

/// One pass over `rest` with the ops in `invalid` left out: the power ops
/// in stamp order against the power state after the cut's, then each
/// non-power op in mainline order against the snapshot's state with the
/// `rest` power ops folded in as the mainline reaches their anchors
/// (`super::resolve_round`, continued).
pub(super) fn rest_round(ctx: &Ctx<'_>, set: &RestSet<'_>, invalid: &[bool]) -> RestRound {
    let n = set.ops.len();
    let p = ctx.cut_applied.len();
    let mut applied = vec![false; n];
    let mut order = Vec::new();
    let (power, rest): (Vec<usize>, Vec<usize>) = (0..n)
        .filter(|&i| !invalid[i])
        .partition(|&i| class_of(set.ops[i].body()) == OpClass::Power);
    let mut power_state = ctx.power_state.clone();
    for i in power {
        if frontier_holds(ctx, set, i) && try_apply(set.ops[i], &mut power_state) {
            applied[i] = true;
            order.push(i);
        }
    }
    let power_order = order.clone();
    let anchor: Vec<Option<usize>> = (0..n)
        .map(|i| {
            (power_order.iter().rposition(|&q| set.past.contains(i, q)))
                .map(|k| p + k)
                .or_else(|| p.checked_sub(1))
        })
        .collect();
    let mut mainline: Vec<(Option<usize>, u64, OpHash, usize)> = rest
        .into_iter()
        .map(|i| (anchor[i], set.ops[i].op().issued_at, set.ops[i].hash(), i))
        .collect();
    mainline.sort();
    let mut state = ctx.snap.state.clone();
    let mut folded = p;
    for (at, _, _, i) in mainline {
        let through = at.map_or(0, |a| a + 1);
        for &q in power_order
            .get(folded - p..through.max(p) - p)
            .unwrap_or_default()
        {
            fold(set.ops[q], &mut state);
        }
        folded = folded.max(through);
        if try_apply(set.ops[i], &mut state) {
            applied[i] = true;
            order.push(i);
        }
    }
    for &q in &power_order[folded - p..] {
        fold(set.ops[q], &mut state);
    }
    RestRound {
        state,
        applied,
        order,
        power_order,
        anchor,
    }
}

/// `super::frontier_holds` for a `rest` op: every cut op of the target is
/// in its causal past under (a′), so the cut index's entry counts them.
fn frontier_holds(ctx: &Ctx<'_>, set: &RestSet<'_>, i: usize) -> bool {
    let body = set.ops[i].body();
    let (Some(claimed), Some(target)) = (target_seq_frontier(body), removal_target(body)) else {
        return true;
    };
    let in_cut = (ctx.snap.cut.authors.get(&target.0)).map_or(0, |(s, _)| s + 1);
    let counted = (0..i)
        .filter(|&o| set.ops[o].author() == target && set.past.contains(i, o))
        .map(|o| set.ops[o].op().seq + 1)
        .fold(in_cut, u64::max);
    claimed == counted
}

/// (d): a non-power op anchored at the cut's last applied power op sorts
/// after every cut non-power op only when its `(issued_at, op_hash)` is
/// above the cut's greatest; one anchored at a `rest` power op sorts after
/// them all.
fn above_the_cut(ctx: &Ctx<'_>, set: &RestSet<'_>, round: &RestRound) -> Result<(), NotOnTop> {
    let p = ctx.cut_applied.len();
    for (i, op) in set.ops.iter().enumerate() {
        if class_of(op.body()) == OpClass::Power || round.anchor[i].is_some_and(|a| a >= p) {
            continue;
        }
        if Some((op.op().issued_at, op.hash())) <= ctx.snap.cut.mainline_max {
            return Err(NotOnTop::BelowTheCut(op.hash()));
        }
    }
    Ok(())
}
