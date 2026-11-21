//! The causal gates between resolution rounds (plan E3.2): strong removal
//! per author (p2panda-auth, S7c.3a), re-join after removal, approval
//! under `ApprovalRequired` (Q6) at the op's anchor, invite proofs
//! against the `InviteCreated`
//! set and its `max_uses` cap, joins backdated before their own deps
//! (SEC-3), and one member per member record.

use std::collections::{BTreeMap, HashMap};

use rekindle_types::governance::{AdmissionMode, GenesisCert, GovernanceEntry, OpHash};
use rekindle_types::id::PseudonymKey;
use rekindle_types::join_refusal::JoinRefusal;

use super::resolve::{Ancestry, OpSet};
use super::Round;
use crate::op::VerifiedOp;
use crate::power::{removal_target, target_seq_frontier};

/// Joins counted per invite id by `invite_gate`.
pub(super) type InviteUses = BTreeMap<[u8; 16], u32>;

/// What the gates decided from one round.
pub(super) struct Gated {
    /// The ops the next round leaves out.
    pub invalid: Vec<bool>,
    /// Per op, the `MemberRemoved` or `BanEntry` that masks it.
    pub masked_by: Vec<Option<usize>>,
    /// The joins `invite_gate` counted on each invite.
    pub uses: InviteUses,
    /// Per op index, why `invite_gate` refused a `MemberJoined`.
    pub invite_refusals: BTreeMap<usize, JoinRefusal>,
    /// Per op index, the member holding the record a `MemberJoined` or
    /// `MemberRecordMoved` refused by `record_gate` named.
    pub record_refusals: BTreeMap<usize, PseudonymKey>,
}

/// The ops the next round leaves out, from what `round` applied.
pub(super) fn invalidated(cert: &GenesisCert, set: &OpSet<'_>, round: &Round) -> Gated {
    let n = set.ops.len();
    let mut invalid = vec![false; n];
    let masked_by = strong_removal(set, round, &mut invalid);
    approval_gate(cert, set, round, &mut invalid);
    backdated_join_gate(set, &mut invalid);
    let mut record_refusals = BTreeMap::new();
    record_gate(cert, set, round, &mut invalid, &mut record_refusals);
    checkpoint_gate(set, round, &mut invalid);
    let mut invite_refusals = BTreeMap::new();
    let uses = invite_gate(set, round, &mut invalid, &mut invite_refusals);
    Gated {
        invalid,
        masked_by,
        uses,
        invite_refusals,
        record_refusals,
    }
}

/// Strong removal per author (plan E3.2 S7c.3a). A `MemberRemoved` or
/// `BanEntry` masks every op of its target at a seq at or above its target
/// seq frontier, whatever deps the op cites: the target's seqs are
/// contiguous along `prev`, so its next op takes the next seq whatever it
/// cites. Its ops below the frontier stand, judged at their own position.
/// A demotion (`RoleUnassignment`) carries no frontier and masks the
/// target's ops concurrent with it or after it. A concurrent removal of
/// the remover by its target (mutual) stands, and a re-join after the
/// removal lifts it for the ops that follow the re-join. Returns, per op,
/// the first removal (in stamp order) that masks it.
fn strong_removal(set: &OpSet<'_>, round: &Round, invalid: &mut [bool]) -> Vec<Option<usize>> {
    let n = set.ops.len();
    let mut masked_by = vec![None; n];
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
            let exempt = if concurrent {
                mutual
            } else {
                rejoined(set, round, r, o)
            };
            if after && !exempt {
                invalid[o] = true;
                if frontier.is_some() && masked_by[o].is_none() {
                    masked_by[o] = Some(r);
                }
            }
        }
    }
    masked_by
}

/// Under `ApprovalRequired`, an op of a member not approved at the op's
/// anchor (the admission decisions among the power ops in its causal
/// past, the last one standing) is refused, its own join and leave aside
/// (plan E3.2 S7c.3a: `admitted` at the op's anchor, not the final one).
fn approval_gate(cert: &GenesisCert, set: &OpSet<'_>, round: &Round, invalid: &mut [bool]) {
    if round.state.effective_admission_mode() != AdmissionMode::ApprovalRequired {
        return;
    }
    let mut decisions: HashMap<&PseudonymKey, Vec<(usize, bool)>> = HashMap::new();
    for (at, &p) in round.power_order.iter().enumerate() {
        match set.ops[p].body() {
            GovernanceEntry::MemberApproved { target } => {
                decisions.entry(target).or_default().push((at, true));
            }
            GovernanceEntry::MemberRejected { target, .. } => {
                decisions.entry(target).or_default().push((at, false));
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
            decisions
                .get(op.author())
                .and_then(|d| d.iter().rev().find(|(at, _)| *at <= anchor))
                .is_some_and(|(_, approved)| *approved)
        });
        if !cert.is_creator(op.author()) && !self_membership && !approved {
            invalid[o] = true;
        }
    }
}

/// A `MemberJoined` whose `issued_at` is earlier than the latest
/// `issued_at` among its deps is refused (register SEC-3): a join can't be
/// dated before its own causal past, so it can't slip under the expiry of
/// an invite revoked or expired by then.
fn backdated_join_gate(set: &OpSet<'_>, invalid: &mut [bool]) {
    for (o, op) in set.ops.iter().enumerate() {
        if !matches!(op.body(), GovernanceEntry::MemberJoined { .. }) {
            continue;
        }
        let latest_dep = set
            .ops
            .iter()
            .filter(|d| op.op().deps.iter().any(|r| r.op_hash == d.hash()))
            .map(|d| d.op().issued_at)
            .max();
        if latest_dep.is_some_and(|at| op.op().issued_at < at) {
            invalid[o] = true;
        }
    }
}

/// A `MemberJoined` or `MemberRecordMoved` naming a member record another
/// member already holds is refused (S7 review). Every binding reaching the
/// merge is proven by the record owner's `ownerProof` (`op::verify`), so
/// only `K_M`'s holder can claim a record and this gate only keeps one
/// holder from binding two pseudonyms to it: a reader accepts data from `M(p)` only under the
/// binding in `p`'s own join (slot-integrity §4.2), so two members bound to
/// one record would let either one's writes pass as the other's. The
/// record goes to the first applied join in the op set's stamp order (the
/// cert's creator holds its own from the start), which every reader
/// computes alike whatever order the ops arrived in; a member's own
/// re-join on its record is not a second claim. Each refused claim is
/// kept in `refusals` with the record's holder, so the claimant's op is
/// quarantined with that reason (D15 "visible reason"), never dropped
/// silently.
fn record_gate(
    cert: &GenesisCert,
    set: &OpSet<'_>,
    round: &Round,
    invalid: &mut [bool],
    refusals: &mut BTreeMap<usize, PseudonymKey>,
) {
    let mut holder: BTreeMap<&str, &PseudonymKey> = BTreeMap::new();
    holder.insert(cert.creator_member_record.as_str(), &cert.creator_pseudonym);
    for (o, op) in set.ops.iter().enumerate() {
        let (GovernanceEntry::MemberJoined { member_record, .. }
        | GovernanceEntry::MemberRecordMoved { member_record, .. }) = op.body()
        else {
            continue;
        };
        match holder.get(member_record.as_str()) {
            Some(&held) if held != op.author() => {
                invalid[o] = true;
                refusals.insert(o, held.clone());
            }
            None if round.applied[o] && !invalid[o] => {
                holder.insert(member_record.as_str(), op.author());
            }
            Some(_) | None => {}
        }
    }
}

/// Whether op `o` (by a member removed at `r`, with `r` in `o`'s past)
/// follows a re-join of that member made after the removal.
fn rejoined(set: &OpSet<'_>, round: &Round, r: usize, o: usize) -> bool {
    let author = set.ops[o].author();
    (0..set.ops.len()).any(|j| {
        round.applied[j]
            && set.ops[j].author() == author
            && matches!(set.ops[j].body(), GovernanceEntry::MemberJoined { .. })
            && set.past.contains(j, r)
            && (j == o || set.past.contains(o, j))
    })
}

/// A `MemberJoined` with an invite proof needs a live `InviteCreated` with
/// that commitment in its causal past (not expired at its `issued_at`),
/// to be in the causal past of every applied `InviteRevoked` of that
/// invite ([`revoke_excluding`]), and fewer than `max_uses` earlier joins
/// on it (`max_uses` is in `1..=MAX_USES_PER_INVITE`; there are no
/// unlimited invites). Returns the joins it counted on each invite;
/// `refusals` gets why each join it refused was refused.
fn invite_gate(
    set: &OpSet<'_>,
    round: &Round,
    invalid: &mut [bool],
    refusals: &mut BTreeMap<usize, JoinRefusal>,
) -> InviteUses {
    let mut uses = InviteUses::new();
    for (o, op) in set.ops.iter().enumerate() {
        let GovernanceEntry::MemberJoined {
            invite_proof: Some(proof),
            ..
        } = op.body()
        else {
            continue;
        };
        let invite = (0..o)
            .filter(|&c| round.applied[c] && set.past.contains(o, c))
            .find_map(|c| match set.ops[c].body() {
                GovernanceEntry::InviteCreated {
                    invite_id,
                    code_hash,
                    max_uses,
                    expires_at,
                    ..
                } if *code_hash == proof.code_hash => Some((*invite_id, *max_uses, *expires_at)),
                _ => None,
            });
        let Some((invite_id, max_uses, expires_at)) = invite else {
            invalid[o] = true;
            refusals.insert(o, JoinRefusal::InviteUnknown);
            continue;
        };
        let revoke = revoke_excluding(&set.ops, &set.past, &round.applied, o, invite_id);
        let expired = expires_at.is_some_and(|exp| op.op().issued_at > exp);
        let used = uses.entry(invite_id).or_insert(0);
        if let Some(refusal) = revoke_refusal(revoke, expired, max_uses) {
            invalid[o] = true;
            refusals.insert(o, refusal);
        } else if *used >= max_uses {
            invalid[o] = true;
            refusals.insert(o, JoinRefusal::InviteUsedUp);
        } else if !invalid[o] {
            *used += 1;
        }
    }
    uses
}

/// The first applied `InviteRevoked` of `invite_id` that does not hold
/// join `o` in its causal past, as the number of joins on that invite its
/// past holds (plan SU1). The inviter that admits the join filling
/// `max_uses` revokes the invite citing it (`bootstrap.rs`), so a join
/// citing stale deps (ground to a lower hash to sort first) can never take
/// a use from a join already admitted; a join concurrent with a manual
/// revoke is refused with it (fail-closed: the revoker did not see it).
pub(super) fn revoke_excluding(
    ops: &[&VerifiedOp],
    past: &Ancestry,
    applied: &[bool],
    o: usize,
    invite_id: [u8; 16],
) -> Option<u32> {
    let GovernanceEntry::MemberJoined {
        invite_proof: Some(proof),
        ..
    } = ops[o].body()
    else {
        return None;
    };
    let v = (0..ops.len()).find(|&v| {
        applied[v]
            && !past.contains(v, o)
            && matches!(ops[v].body(), GovernanceEntry::InviteRevoked { invite_id: id } if *id == invite_id)
    })?;
    let joins = (0..v)
        .filter(|&j| applied[j] && past.contains(v, j))
        .filter(|&j| {
            matches!(ops[j].body(), GovernanceEntry::MemberJoined { invite_proof: Some(p), .. }
                if p.code_hash == proof.code_hash)
        })
        .count();
    Some(u32::try_from(joins).unwrap_or(u32::MAX))
}

/// Why a join on an invite is refused for a revoke (`revoke`, the joins
/// in that revoke's past, from [`revoke_excluding`]) or its expiry:
/// `InviteUsedUp` when the revoke's past holds `max_uses` joins, else
/// `InviteNoLongerValid` (one reason for a revoke and an expiry,
/// `rekindle_types::join_refusal`).
pub(super) fn revoke_refusal(
    revoke: Option<u32>,
    expired: bool,
    max_uses: u32,
) -> Option<JoinRefusal> {
    match revoke {
        Some(joins) if joins >= max_uses => Some(JoinRefusal::InviteUsedUp),
        Some(_) => Some(JoinRefusal::InviteNoLongerValid),
        None => expired.then_some(JoinRefusal::InviteNoLongerValid),
    }
}

/// A `Checkpoint` past height 1 needs its `prevCheckpoint` applied, one
/// height below it, and the previous cut inside its own: every previous
/// cut head is one of its cut heads or in the causal past of one (plan
/// E3.2 S7c.3).
fn checkpoint_gate(set: &OpSet<'_>, round: &Round, invalid: &mut [bool]) {
    let mut index: Option<HashMap<OpHash, usize>> = None;
    for (o, op) in set.ops.iter().enumerate() {
        let GovernanceEntry::Checkpoint(c) = op.body() else {
            continue;
        };
        let Some(prev) = &c.prev else {
            continue;
        };
        let index = index.get_or_insert_with(|| {
            (set.ops.iter().enumerate())
                .map(|(i, op)| (op.hash(), i))
                .collect()
        });
        let at = |hash: &OpHash| index.get(hash).copied();
        let heads: Vec<usize> = c.cut.iter().filter_map(|r| at(&r.op_hash)).collect();
        let covered = |h: usize| heads.iter().any(|&k| k == h || set.past.contains(k, h));
        let holds = at(&prev.op_hash).is_some_and(|p| {
            round.applied[p]
                && matches!(set.ops[p].body(), GovernanceEntry::Checkpoint(pc)
                    if pc.height.checked_add(1) == Some(c.height)
                        && pc.cut.iter().all(|r| at(&r.op_hash).is_some_and(covered)))
        });
        invalid[o] |= heads.len() != c.cut.len() || !holds;
    }
}
