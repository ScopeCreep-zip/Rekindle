//! `gates::invalidated` over `rest` alone (plan E3.2 S7c.4a). Each gate
//! asks of a `rest` op what the full merge asks of it; under (a′) every
//! cut op is in its causal past, so what the gate reads from the cut comes
//! from the snapshot: strong-removal masks, the admission decisions among
//! the cut's power ops, record holders, invite codes and `uses`.

use std::collections::{BTreeMap, HashMap};

use rekindle_types::governance::{AdmissionMode, GovernanceEntry, OpHash};

use super::resume::{Ctx, Dep, NotOnTop, RestRound, RestSet};
use crate::power::{removal_target, target_seq_frontier};

/// What the gates decided from one pass.
pub(super) struct Gated {
    pub invalid: Vec<bool>,
    /// Per op, the removal (by hash) that masks it.
    pub masked_by: Vec<Option<OpHash>>,
    /// The joins counted on each invite, the snapshot's `uses` included.
    pub uses: BTreeMap<[u8; 16], u32>,
}

pub(super) fn invalidated(
    ctx: &Ctx<'_>,
    set: &RestSet<'_>,
    round: &RestRound,
) -> Result<Gated, NotOnTop> {
    let mut invalid = vec![false; set.ops.len()];
    let masked_by = strong_removal(ctx, set, round, &mut invalid)?;
    approval_gate(ctx, set, round, &mut invalid);
    backdated_join_gate(ctx, set, &mut invalid)?;
    record_gate(ctx, set, round, &mut invalid);
    checkpoint_gate(ctx, set, round, &mut invalid)?;
    let uses = invite_gate(ctx, set, round, &mut invalid)?;
    Ok(Gated {
        invalid,
        masked_by,
        uses,
    })
}

/// Whether a re-join could exempt `o` from a removal (`after`: a `rest`
/// removal in the re-join's past; `None`: a cut one, in every `rest` op's
/// past). `gates::rejoined` reads whether that `MemberJoined` applied in
/// the round, and an op left out of one round is never applied in the
/// next, so the full merge's fixpoint there depends on its earlier rounds,
/// whose cut part the snapshot does not carry: undecidable.
fn rejoin_undecidable(set: &RestSet<'_>, o: usize, after: Option<usize>) -> Result<(), NotOnTop> {
    let author = set.ops[o].author();
    let rejoin = (0..set.ops.len()).any(|j| {
        set.ops[j].author() == author
            && matches!(set.ops[j].body(), GovernanceEntry::MemberJoined { .. })
            && after.is_none_or(|r| set.past.contains(j, r))
            && (j == o || set.past.contains(o, j))
    });
    if rejoin {
        return Err(NotOnTop::Undecidable(set.ops[o].hash()));
    }
    Ok(())
}

/// `gates::strong_removal`: the cut's unlifted removals mask every op of
/// their target (a re-join after one is undecidable); `rest`'s removals
/// mask as in the full merge.
fn strong_removal(
    ctx: &Ctx<'_>,
    set: &RestSet<'_>,
    round: &RestRound,
    invalid: &mut [bool],
) -> Result<Vec<Option<OpHash>>, NotOnTop> {
    let n = set.ops.len();
    let mut masked_by = vec![None; n];
    for o in 0..n {
        if let Some(cause) = ctx.snap.cut.masked.get(&set.ops[o].author().0) {
            rejoin_undecidable(set, o, None)?;
            invalid[o] = true;
            masked_by[o] = *cause;
        }
    }
    let removals = (0..n)
        .filter(|&r| round.applied[r])
        .filter(|&r| removal_target(set.ops[r].body()).is_some_and(|t| t != set.ops[r].author()));
    for r in removals {
        let remover = set.ops[r].author();
        let body = set.ops[r].body();
        let Some(target) = removal_target(body) else {
            continue;
        };
        let frontier = target_seq_frontier(body);
        for o in (0..n).filter(|&o| set.ops[o].author() == target) {
            let mutual = removal_target(set.ops[o].body()) == Some(remover);
            let concurrent = set.past.concurrent(o, r);
            let after = match frontier {
                Some(frontier) => set.ops[o].op().seq >= frontier,
                None => concurrent || set.past.contains(o, r),
            };
            if after && !concurrent {
                rejoin_undecidable(set, o, Some(r))?;
            }
            if after && !(concurrent && mutual) {
                invalid[o] = true;
                if frontier.is_some() && masked_by[o].is_none() {
                    masked_by[o] = Some(set.ops[r].hash());
                }
            }
        }
    }
    Ok(masked_by)
}

/// `gates::approval_gate` over the full power order: the cut's applied
/// power ops, then `rest`'s.
fn approval_gate(ctx: &Ctx<'_>, set: &RestSet<'_>, round: &RestRound, invalid: &mut [bool]) {
    if round.state.effective_admission_mode() != AdmissionMode::ApprovalRequired {
        return;
    }
    let power =
        (ctx.cut_applied.iter().copied()).chain(round.power_order.iter().map(|&q| set.ops[q]));
    let mut decisions: HashMap<[u8; 32], Vec<(usize, bool)>> = HashMap::new();
    for (at, op) in power.enumerate() {
        match op.body() {
            GovernanceEntry::MemberApproved { target } => {
                decisions.entry(target.0).or_default().push((at, true));
            }
            GovernanceEntry::MemberRejected { target, .. } => {
                decisions.entry(target.0).or_default().push((at, false));
            }
            _ => {}
        }
    }
    for (o, op) in set.ops.iter().enumerate() {
        let self_membership = matches!(
            op.body(),
            GovernanceEntry::MemberJoined { .. } | GovernanceEntry::MemberLeft { .. }
        );
        let approved = round.anchor[o].is_some_and(|anchor| {
            (decisions.get(&op.author().0))
                .and_then(|d| d.iter().rev().find(|(at, _)| *at <= anchor))
                .is_some_and(|(_, approved)| *approved)
        });
        if op.author() != &ctx.cert.creator_pseudonym && !self_membership && !approved {
            invalid[o] = true;
        }
    }
}

/// `gates::backdated_join_gate`: a dep in `rest` or a cut power op has a
/// known `issued_at`; any other cut dep is a non-power cut op, at or below
/// the cut's greatest non-power `issued_at`, so it decides nothing when
/// the join is not below that, and is undecidable when it is.
fn backdated_join_gate(
    ctx: &Ctx<'_>,
    set: &RestSet<'_>,
    invalid: &mut [bool],
) -> Result<(), NotOnTop> {
    for (o, op) in set.ops.iter().enumerate() {
        if !matches!(op.body(), GovernanceEntry::MemberJoined { .. }) {
            continue;
        }
        let issued = op.op().issued_at;
        let mut latest = None;
        for (dep, held) in op.op().deps.iter().zip(&set.deps[o]) {
            let at = match held {
                Dep::Rest(i) => set.ops[*i].op().issued_at,
                Dep::Cut => match ctx.cut_power.get(&dep.op_hash) {
                    Some(p) => p.op().issued_at,
                    None if ctx.snap.cut.mainline_max.is_some_and(|(m, _)| m <= issued) => continue,
                    None => return Err(NotOnTop::Undecidable(op.hash())),
                },
            };
            latest = latest.max(Some(at));
        }
        if latest.is_some_and(|at| issued < at) {
            invalid[o] = true;
        }
    }
    Ok(())
}

/// `gates::record_gate`, continuing from the cut's holders.
fn record_gate(ctx: &Ctx<'_>, set: &RestSet<'_>, round: &RestRound, invalid: &mut [bool]) {
    let mut holder = ctx.snap.cut.holders.clone();
    for (o, op) in set.ops.iter().enumerate() {
        let (GovernanceEntry::MemberJoined { member_record, .. }
        | GovernanceEntry::MemberRecordMoved { member_record, .. }) = op.body()
        else {
            continue;
        };
        match holder.get(member_record) {
            Some(held) => invalid[o] |= *held != op.author().0,
            None if round.applied[o] && !invalid[o] => {
                holder.insert(member_record.clone(), op.author().0);
            }
            None => {}
        }
    }
}

/// `gates::checkpoint_gate` for a `rest` `Checkpoint`. A cut head is in
/// the past of every `rest` head; a previous checkpoint in the cut is
/// judged from the cut index's applied checkpoints (plan E3.2 S7c.5);
/// whether one cut op is in another's past the snapshot does not say, so
/// that, and a previous checkpoint the index does not list, is undecidable.
fn checkpoint_gate(
    ctx: &Ctx<'_>,
    set: &RestSet<'_>,
    round: &RestRound,
    invalid: &mut [bool],
) -> Result<(), NotOnTop> {
    let in_cut = |r: &rekindle_types::governance::OpRef| {
        (ctx.snap.cut.authors.get(&r.author.0)).is_some_and(|(s, _)| r.author_seq <= *s)
    };
    let at = |r: &rekindle_types::governance::OpRef| match set.index.get(&r.op_hash) {
        Some(&i) => Some(Dep::Rest(i)),
        None => in_cut(r).then_some(Dep::Cut),
    };
    for (o, op) in set.ops.iter().enumerate() {
        let GovernanceEntry::Checkpoint(c) = op.body() else {
            continue;
        };
        let held = c.cut.iter().all(|r| at(r).is_some());
        let Some(prev) = &c.prev else {
            invalid[o] |= !held;
            continue;
        };
        let holds = match at(prev) {
            Some(Dep::Cut) => {
                // An applied previous checkpoint in the cut is in the cut
                // index with its height (else undecidable). Its cut heads
                // are its deps, so cut ops: `c` covers them when one of its
                // heads is in `rest` (the whole cut in its past, (a′)) or
                // its heads include every head of the snapshot's cut.
                let Some(&height) = ctx.snap.cut.checkpoints.get(&prev.op_hash) else {
                    return Err(NotOnTop::Undecidable(op.hash()));
                };
                let covered = c.cut.iter().any(|k| matches!(at(k), Some(Dep::Rest(_))))
                    || (ctx.snap.heads.iter()).all(|h| c.cut.iter().any(|k| k.op_hash == *h));
                if !covered {
                    return Err(NotOnTop::Undecidable(op.hash()));
                }
                height.checked_add(1) == Some(c.height)
            }
            None => false,
            Some(Dep::Rest(p)) => {
                let GovernanceEntry::Checkpoint(pc) = set.ops[p].body() else {
                    invalid[o] = true;
                    continue;
                };
                let mut covered = true;
                for h in &pc.cut {
                    covered &= covers(set, c, h, &at).ok_or(NotOnTop::Undecidable(op.hash()))?;
                }
                round.applied[p] && pc.height.checked_add(1) == Some(c.height) && covered
            }
        };
        invalid[o] |= !held || !holds;
    }
    Ok(())
}

/// Whether a head of `c` is `h` or has it in its causal past; `None` when
/// that is a question between two cut ops.
fn covers(
    set: &RestSet<'_>,
    c: &rekindle_types::governance::Checkpoint,
    h: &rekindle_types::governance::OpRef,
    at: &impl Fn(&rekindle_types::governance::OpRef) -> Option<Dep>,
) -> Option<bool> {
    let Some(h_at) = at(h) else {
        return Some(false);
    };
    let mut undecided = false;
    for k in &c.cut {
        if k.op_hash == h.op_hash {
            return Some(true);
        }
        match (at(k), h_at) {
            (Some(Dep::Rest(_)), Dep::Cut) => return Some(true),
            (Some(Dep::Rest(k)), Dep::Rest(h)) if set.past.contains(k, h) => return Some(true),
            (Some(Dep::Cut), Dep::Cut) => undecided = true,
            _ => {}
        }
    }
    (!undecided).then_some(false)
}

/// `gates::invite_gate`, continuing from the snapshot's `uses`: the first
/// applied `InviteCreated` in a join's past with its code is the cut's
/// (the cut index's `invite_codes`) when the cut holds one, else the first
/// in `rest`.
fn invite_gate(
    ctx: &Ctx<'_>,
    set: &RestSet<'_>,
    round: &RestRound,
    invalid: &mut [bool],
) -> Result<BTreeMap<[u8; 16], u32>, NotOnTop> {
    let state = &ctx.snap.state;
    let mut uses: BTreeMap<[u8; 16], u32> = (state.invites.iter())
        .map(|(id, invite)| (*id, invite.uses))
        .collect();
    for (o, op) in set.ops.iter().enumerate() {
        let GovernanceEntry::MemberJoined {
            invite_proof: Some(proof),
            ..
        } = op.body()
        else {
            continue;
        };
        let applied_before = |c: usize| c < o && round.applied[c] && set.past.contains(o, c);
        let invite = match ctx.snap.cut.invite_codes.get(&proof.code_hash) {
            Some((id, true)) => Some((*id, 0, None, true)),
            Some((id, false)) => {
                let live = state
                    .invites
                    .get(id)
                    .ok_or(NotOnTop::Undecidable(op.hash()))?;
                Some((*id, live.max_uses, live.expires_at, false))
            }
            None => (0..o)
                .filter(|&c| applied_before(c))
                .find_map(|c| match set.ops[c].body() {
                    GovernanceEntry::InviteCreated {
                        invite_id,
                        code_hash,
                        max_uses,
                        expires_at,
                        ..
                    } if *code_hash == proof.code_hash => {
                        Some((*invite_id, *max_uses, *expires_at, false))
                    }
                    _ => None,
                }),
        };
        let Some((invite_id, max_uses, expires_at, revoked_in_cut)) = invite else {
            invalid[o] = true;
            continue;
        };
        let revoked = revoked_in_cut
            || (0..o).any(|v| {
                applied_before(v)
                    && matches!(set.ops[v].body(), GovernanceEntry::InviteRevoked { invite_id: id } if *id == invite_id)
            });
        let expired = expires_at.is_some_and(|exp| op.op().issued_at > exp);
        let used = uses.entry(invite_id).or_insert(0);
        if revoked || expired || *used >= max_uses {
            invalid[o] = true;
        } else if !invalid[o] {
            *used += 1;
        }
    }
    Ok(uses)
}
