//! The causal gates between resolution rounds (plan E3.2): strong removal
//! (p2panda-auth), re-join after removal, approval under
//! `ApprovalRequired` (Q6), invite proofs against the `InviteCreated`
//! set and its `max_uses` cap, joins backdated before their own deps
//! (SEC-3), and one member per member record.

use std::collections::BTreeMap;

use rekindle_types::governance::{AdmissionMode, GenesisCert, GovernanceEntry};
use rekindle_types::id::PseudonymKey;

use super::resolve::OpSet;
use super::Round;
use crate::power::removal_target;

/// The ops the next round leaves out, from what `round` applied.
pub(super) fn invalidated(cert: &GenesisCert, set: &OpSet<'_>, round: &Round) -> Vec<bool> {
    let n = set.ops.len();
    let mut invalid = vec![false; n];
    let removals: Vec<usize> = (0..n)
        .filter(|&r| round.applied[r])
        .filter(|&r| removal_target(set.ops[r].body()).is_some_and(|t| t != set.ops[r].author()))
        .collect();
    for &r in &removals {
        let remover = set.ops[r].author();
        let Some(target) = removal_target(set.ops[r].body()) else {
            continue;
        };
        for o in (0..n).filter(|&o| set.ops[o].author() == target) {
            let mutual = removal_target(set.ops[o].body()) == Some(remover);
            if set.past.concurrent(o, r) {
                invalid[o] |= !mutual;
            } else if set.past.contains(o, r) && !rejoined(set, round, r, o) {
                invalid[o] = true;
            }
        }
    }
    if round.state.effective_admission_mode() == AdmissionMode::ApprovalRequired {
        for (o, op) in set.ops.iter().enumerate() {
            let self_membership = matches!(
                op.body(),
                GovernanceEntry::MemberJoined { .. } | GovernanceEntry::MemberLeft { .. }
            );
            let approved = round
                .state
                .admitted
                .get(op.author())
                .is_some_and(|d| d.approved);
            if op.author() != &cert.creator_pseudonym && !self_membership && !approved {
                invalid[o] = true;
            }
        }
    }
    backdated_join_gate(set, &mut invalid);
    record_gate(cert, set, round, &mut invalid);
    invite_gate(set, round, &mut invalid);
    invalid
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
/// re-join on its record is not a second claim.
fn record_gate(cert: &GenesisCert, set: &OpSet<'_>, round: &Round, invalid: &mut [bool]) {
    let mut holder: BTreeMap<&str, &PseudonymKey> = BTreeMap::new();
    holder.insert(cert.creator_member_record.as_str(), &cert.creator_pseudonym);
    for (o, op) in set.ops.iter().enumerate() {
        let (GovernanceEntry::MemberJoined { member_record, .. }
        | GovernanceEntry::MemberRecordMoved { member_record, .. }) = op.body()
        else {
            continue;
        };
        match holder.get(member_record.as_str()) {
            Some(&held) => invalid[o] |= held != op.author(),
            None if round.applied[o] && !invalid[o] => {
                holder.insert(member_record.as_str(), op.author());
            }
            None => {}
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
/// that commitment in its causal past (not revoked there, not expired at
/// its `issued_at`), and fewer than `max_uses` earlier joins on it
/// (`max_uses` is in `1..=MAX_USES_PER_INVITE`; there are no unlimited
/// invites).
fn invite_gate(set: &OpSet<'_>, round: &Round, invalid: &mut [bool]) {
    let mut uses: BTreeMap<[u8; 16], u32> = BTreeMap::new();
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
            continue;
        };
        let revoked = (0..o).any(|v| {
            round.applied[v]
                && set.past.contains(o, v)
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
}
