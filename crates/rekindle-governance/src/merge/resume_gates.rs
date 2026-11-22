//! `gates::invalidated` over `rest` alone (plan E3.2 S7c.4a). Each gate
//! asks of a `rest` op what the full merge asks of it; under (a′) every
//! cut op is in its causal past, so what the gate reads from the cut comes
//! from the snapshot: strong-removal masks, the admission decisions among
//! the cut's power ops, record holders, invite codes and `uses`, and the
//! applied successor offers (`successors`, S7s.3e).

use rekindle_types::join_refusal::JoinRefusal;
use std::collections::{BTreeMap, HashMap};

use rekindle_types::governance::{AdmissionMode, GovernanceEntry, OpHash};
use rekindle_types::id::PseudonymKey;

use super::resume::{Ctx, Dep, NotOnTop, RestRound, RestSet};
use crate::link_round::{
    acceptance_links_cited, answers_request, cited_offer, fits_offer, LinkRoundBody,
};
use crate::power::{removal_target, target_seq_frontier};

/// What the gates decided from one pass.
pub(super) struct Gated {
    pub invalid: Vec<bool>,
    /// Per op, the removal (by hash) that masks it.
    pub masked_by: Vec<Option<OpHash>>,
    /// The joins counted on each invite, the snapshot's `uses` included.
    pub uses: BTreeMap<[u8; 16], u32>,
    /// Per op index, why `invite_gate` refused a `MemberJoined`.
    pub invite_refusals: BTreeMap<usize, JoinRefusal>,
    /// Per op index, the member holding the record a claim refused by
    /// `record_gate` named (`gates::Gated::record_refusals`).
    pub record_refusals: BTreeMap<usize, PseudonymKey>,
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
    let mut record_refusals = BTreeMap::new();
    record_gate(ctx, set, round, &mut invalid, &mut record_refusals)?;
    checkpoint_gate(ctx, set, round, &mut invalid)?;
    successor_gate(ctx, set, round, &mut invalid)?;
    link_gate(ctx, set, round, &mut invalid);
    beside_revoke_gate(ctx, set, round)?;
    let mut invite_refusals = BTreeMap::new();
    let uses = invite_gate(ctx, set, round, &mut invalid, &mut invite_refusals)?;
    Ok(Gated {
        invalid,
        masked_by,
        uses,
        invite_refusals,
        record_refusals,
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
    // A `MemberLeft` is not a power op: one in the cut is read from the
    // snapshot's `departed_heads` (every op of its author in `rest` is at a
    // higher seq, the author's ops chaining along `prev`), one in `rest`
    // from the round, as `gates::approval_gate` reads it.
    let mut left = HashMap::new();
    for (o, op) in set.ops.iter().enumerate() {
        if round.applied[o] && matches!(op.body(), GovernanceEntry::MemberLeft { .. }) {
            super::gates::note_leave(&mut left, op.author(), op.op().seq);
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
        let seated = !ctx.snap.state.departed_heads.contains_key(op.author())
            && super::gates::seated_at(ctx.cert, &left, op.author(), op.op().seq);
        if !seated && !self_membership && !approved {
            invalid[o] = true;
        }
    }
}

/// `gates::backdated_join_gate`: a dep in `rest` or a supplied (applied)
/// cut power op has a known `issued_at`; a refused cut power op's is not
/// known (only applied ones are fetched), so a join citing one is
/// undecidable (S7c.8: the property test found a join dated below a
/// refused power head it cites, refused by the full merge, applied by the
/// resume); any other cut dep is a non-power cut op, at or below the
/// cut's greatest non-power `issued_at`, so it decides nothing when the
/// join is not below that, and is undecidable when it is.
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
                    None if (ctx.snap.cut.power.iter()).any(|(c, _)| c.hash == dep.op_hash) => {
                        return Err(NotOnTop::Undecidable(op.hash()));
                    }
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

/// `gates::record_gate`, continuing from the cut's holders. A join beside
/// the cut naming a record another member holds in the cut is not on top:
/// the record goes to the first applied claim in stamp order, and the
/// cut's claim may sort after it (plan E3.2 S7c.8).
fn record_gate(
    ctx: &Ctx<'_>,
    set: &RestSet<'_>,
    round: &RestRound,
    invalid: &mut [bool],
    refusals: &mut BTreeMap<usize, PseudonymKey>,
) -> Result<(), NotOnTop> {
    let mut holder = ctx.snap.cut.holders.clone();
    for (o, op) in set.ops.iter().enumerate() {
        let (GovernanceEntry::MemberJoined { member_record, .. }
        | GovernanceEntry::MemberRecordMoved { member_record, .. }) = op.body()
        else {
            continue;
        };
        let in_cut = ctx.snap.cut.holders.get(member_record);
        if set.reach[o].is_some() && in_cut.is_some_and(|held| *held != op.author().0) {
            return Err(NotOnTop::BesideTheCut(op.hash()));
        }
        match holder.get(member_record) {
            Some(held) if *held != op.author().0 => {
                invalid[o] = true;
                refusals.insert(o, PseudonymKey(*held));
            }
            None if round.applied[o] && !invalid[o] => {
                holder.insert(member_record.clone(), op.author().0);
            }
            Some(_) | None => {}
        }
    }
    Ok(())
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
                let Some(height) = ctx.snap.cut.checkpoint_height(&prev.op_hash) else {
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
/// in `rest`. A join beside the cut on a cut invite is not on top when its
/// deps do not reach the `InviteCreated` (the full merge may find none in
/// its past), when the cut revokes the invite (the revocation may be
/// concurrent with it), or when the invite has no use left for it (the
/// full merge counts joins in stamp order, and a cut join may sort after
/// it); with a use left for every such join, no count reaches `max_uses`
/// in any order, so the cut's joins keep their verdicts (plan E3.2 S7c.8).
fn invite_gate(
    ctx: &Ctx<'_>,
    set: &RestSet<'_>,
    round: &RestRound,
    invalid: &mut [bool],
    refusals: &mut BTreeMap<usize, JoinRefusal>,
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
        let in_cut = ctx.snap.cut.invite_codes.get(&proof.code_hash);
        let beside = set.reach[o].as_ref();
        if let (Some(reach), Some((_, revoked, created))) = (beside, in_cut) {
            if *revoked || !super::resume::reaches(reach, &created.author, created.seq) {
                return Err(NotOnTop::BesideTheCut(op.hash()));
            }
        }
        let invite = match in_cut {
            Some((id, true, _)) => Some((*id, 0, None, true)),
            Some((id, false, _)) => {
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
            refusals.insert(o, JoinRefusal::InviteUnknown);
            continue;
        };
        let below_cut = live_uses(state, invite_id);
        let revoke =
            super::gates::revoke_excluding(&set.ops, &set.past, &round.applied, o, invite_id)
                .map(|joins| joins.saturating_add(below_cut));
        let revoked = revoked_in_cut || revoke.is_some();
        let expired = expires_at.is_some_and(|exp| op.op().issued_at > exp);
        let used = uses.entry(invite_id).or_insert(0);
        if beside.is_some() && in_cut.is_some() && !revoked && !expired && *used >= max_uses {
            return Err(NotOnTop::BesideTheCut(op.hash()));
        }
        let refusal = if revoked_in_cut {
            Some(JoinRefusal::InviteNoLongerValid)
        } else {
            super::gates::revoke_refusal(revoke, expired, max_uses)
        };
        if let Some(refusal) = refusal {
            invalid[o] = true;
            refusals.insert(o, refusal);
        } else if *used >= max_uses {
            invalid[o] = true;
            refusals.insert(o, JoinRefusal::InviteUsedUp);
        } else if !invalid[o] {
            *used += 1;
        }
    }
    Ok(uses)
}

/// SU1 refuses a join outside the causal past of any applied
/// `InviteRevoked` of its invite (`gates::revoke_excluding`), a cut join
/// included. A revoke with every cut head in its past holds every cut join;
/// one beside the cut (S7c.8) may not, and the snapshot carries only the
/// count of the cut's joins, not which ops they are, so whether the full
/// merge refuses a cut join (and changes the cut's membership) is not
/// decidable from it: not on top when the cut holds the invite and may
/// have admitted a join on it (revoked in the cut, or live with a use).
fn beside_revoke_gate(ctx: &Ctx<'_>, set: &RestSet<'_>, round: &RestRound) -> Result<(), NotOnTop> {
    for (v, op) in set.ops.iter().enumerate() {
        let GovernanceEntry::InviteRevoked { invite_id } = op.body() else {
            continue;
        };
        if set.reach[v].is_none() || !round.applied[v] {
            continue;
        }
        let in_cut = (ctx.snap.cut.invite_codes.values()).find(|(id, _, _)| id == invite_id);
        if let Some((_, revoked, _)) = in_cut {
            if *revoked || live_uses(&ctx.snap.state, *invite_id) > 0 {
                return Err(NotOnTop::BesideTheCut(op.hash()));
            }
        }
    }
    Ok(())
}

/// The joins the snapshot counted on `invite_id` below the cut (none for an
/// invite the cut does not hold live): a revoke after the cut that applies
/// here has the cut's joins in its past (`beside_revoke_gate`).
fn live_uses(state: &crate::state::GovernanceState, invite_id: [u8; 16]) -> u32 {
    state.invites.get(&invite_id).map_or(0, |i| i.uses)
}

/// `gates::successor_gate` for a `rest` acceptance. An offer in the cut is
/// read from the snapshot's `successors` (the cut's applied successor
/// ops); a later offer or withdrawal of its author there supersedes it, as
/// every cut op is in a `rest` op's past (a′). For an op beside the cut,
/// whose past may hold only part of the cut, such a later one is
/// undecidable. The link round an acceptance cites is read from
/// `round.state`, the cut's `successor_links` with the rest's applied.
fn successor_gate(
    ctx: &Ctx<'_>,
    set: &RestSet<'_>,
    round: &RestRound,
    invalid: &mut [bool],
) -> Result<(), NotOnTop> {
    let cut = &ctx.snap.state.successors;
    for (o, op) in set.ops.iter().enumerate() {
        let Some(accepts) = cited_offer(op.body()) else {
            continue;
        };
        let in_deps = op.op().deps.iter().any(|r| r == accepts);
        let later_in_rest = |offerer: &PseudonymKey, p: Option<usize>| {
            (0..set.ops.len()).any(|c| {
                Some(c) != p
                    && round.applied[c]
                    && set.ops[c].author() == offerer
                    && set.past.contains(o, c)
                    && p.is_none_or(|p| set.past.contains(c, p))
                    && matches!(set.ops[c].body(),
                        GovernanceEntry::CommunitySuccessor(s) if s.is_offer_or_withdrawal())
            })
        };
        let holds = if let Some(&p) = set.index.get(&accepts.op_hash) {
            round.applied[p]
                && !later_in_rest(set.ops[p].author(), Some(p))
                && matches!(set.ops[p].body(), GovernanceEntry::CommunitySuccessor(offer)
                    if fits_offer(op.author(), op.body(), offer))
        } else if let Some(row) = cut.get(&accepts.op_hash) {
            let later_in_cut = cut.values().any(|s| {
                s.author == row.author
                    && s.stamp.lamport > row.stamp.lamport
                    && s.successor.is_offer_or_withdrawal()
            });
            if later_in_cut && set.reach[o].is_some() {
                return Err(NotOnTop::Undecidable(op.hash()));
            }
            !later_in_cut
                && !later_in_rest(&row.author, None)
                && fits_offer(op.author(), op.body(), &row.successor)
        } else {
            false
        };
        let linked = acceptance_links_cited(op.author(), op.body(), &op.op().deps, &round.state);
        // Lead decision (55), as `gates::successor_gate`: an acceptance of
        // the same offer in the cut (the snapshot's `successors`) is the
        // same author's earlier op, so in this one's past; one in the rest
        // counts when it is in its past.
        let again = matches!(op.body(), GovernanceEntry::CommunitySuccessor(_))
            && (cut.values().any(|s| {
                s.successor
                    .accepts
                    .as_ref()
                    .is_some_and(|r| r.op_hash == accepts.op_hash)
            }) || (0..set.ops.len()).any(|c| {
                c != o
                    && round.applied[c]
                    && set.past.contains(o, c)
                    && super::gates::accepts_same(set.ops[c].body(), accepts)
            }));
        invalid[o] |= !in_deps || !holds || !linked || again;
    }
    Ok(())
}

/// `gates::link_gate` for a `rest` link: its request is in the rest
/// (applied) or in the cut (the snapshot's `successor_links`).
fn link_gate(ctx: &Ctx<'_>, set: &RestSet<'_>, round: &RestRound, invalid: &mut [bool]) {
    let cut = &ctx.snap.state.successor_links;
    for (o, op) in set.ops.iter().enumerate() {
        let GovernanceEntry::SuccessorLink(link) = op.body() else {
            continue;
        };
        let in_deps = op.op().deps.contains(&link.request);
        let holds = if let Some(&q) = set.index.get(&link.request.op_hash) {
            round.applied[q]
                && matches!(set.ops[q].body(), GovernanceEntry::SuccessorLinkRequest(r)
                    if answers_request(op.author(), link, r))
        } else {
            cut.get(&link.request.op_hash).is_some_and(|row| {
                matches!(&row.body, LinkRoundBody::Request(r) if answers_request(op.author(), link, r))
            })
        };
        invalid[o] |= !in_deps || !holds;
    }
}
