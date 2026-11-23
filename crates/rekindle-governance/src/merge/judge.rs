//! The per-op device-key judge of `prepare` (ADR 0022 rules 1, 2, 6 and
//! 10; plan E3.2 S7d-keys K4b.1): every op's device key is judged for its
//! author at the op's causal deps, so `op.author` is bound to the key that
//! signed it (`VerifiedOp::verify_with` checks only that the key signed).
//!
//! The author's view at an op's deps is built from the author's
//! **carriers** in the op's causal past — its `MemberJoined`s, `DeviceKey`s
//! and `DeviceRetired`s, and the `DeviceReadmit`s naming it — absorbed in
//! stamp order over the cert's seed for the creator: a pure function of the
//! op's past (D15 "pure effects"), so every reader judges each op alike in
//! any arrival order. Views are memoised by (author, carriers held).

use std::collections::HashMap;
use std::rc::Rc;

use rekindle_types::governance::{GenesisCert, GovernanceEntry};
use rekindle_types::id::PseudonymKey;

use super::resolve::Ancestry;
use super::resume::{reaches, Reach};
use crate::device_judge::{judge_op_at, DeviceView, KeyJudgement, MemberDevices};
use crate::op::VerifiedOp;

/// The member whose view `op` carries device elements for, if any: its
/// author for a join, endorsement or retirement, the subject of a readmit.
pub(super) fn carrier_of(op: &VerifiedOp) -> Option<&PseudonymKey> {
    match op.body() {
        GovernanceEntry::MemberJoined { .. }
        | GovernanceEntry::DeviceKey(_)
        | GovernanceEntry::DeviceRetired(_) => Some(op.author()),
        GovernanceEntry::DeviceReadmit(r) => Some(&r.member),
        _ => None,
    }
}

/// `author`'s elements over `base` (the cert's seed for the creator, or a
/// snapshot's entry) with `carriers`, `author`'s carriers in stamp order,
/// absorbed, each with what it saw of the key a `DeviceRetired` retires
/// ([`DeviceView::harvest_seen`]).
pub(super) fn member_view<'v>(
    cert: &GenesisCert,
    anchor: &str,
    author: &PseudonymKey,
    base: Option<&MemberDevices>,
    carriers: impl Iterator<Item = (&'v VerifiedOp, Option<u64>)>,
) -> MemberDevices {
    let mut view = DeviceView::default();
    match base {
        Some(md) => {
            view.members.insert(author.clone(), md.clone());
        }
        None if *author == cert.creator_pseudonym => view.seed_genesis(anchor, cert),
        None => {}
    }
    for (op, seen) in carriers {
        view.harvest_seen(op, seen);
    }
    view.members.remove(author).unwrap_or_default()
}

/// Whether `op` is signed by its author's pseudonym: the interim author
/// binding (lead decision K4b Q1). The local signer still signs governance
/// ops with the pseudonym; such an op is author-bound by its signature
/// alone. Removed at plan E3.2 S7d-keys.5, when every op is signed by an
/// endorsed device key and `p` signs only endorsements (ADR 0022 rule 1).
pub(super) fn pseudonym_signed(op: &VerifiedOp) -> bool {
    op.op().device_key == op.author().0
}

/// `prepare`'s judge: the carriers seen so far and the memoised views.
pub(super) struct Judge<'c> {
    cert: &'c GenesisCert,
    /// Per member: the kept indices of its carriers, in stamp order.
    carriers: HashMap<PseudonymKey, Vec<usize>>,
    /// Per `DeviceRetired` carrier's index: the retired key's highest seq
    /// in its strict causal past (its carrier floor, rule 5).
    seen: HashMap<usize, u64>,
    memo: HashMap<(PseudonymKey, Vec<usize>), Rc<MemberDevices>>,
}

impl<'c> Judge<'c> {
    pub(super) fn new(cert: &'c GenesisCert) -> Self {
        Self {
            cert,
            carriers: HashMap::new(),
            seen: HashMap::new(),
            memo: HashMap::new(),
        }
    }

    /// Judge `op`, whose deps are the kept ops `deps` (indices into
    /// `kept`, whose causal pasts `past` holds): an op whose key is not
    /// its author's pseudonym must be judged effective for that author.
    pub(super) fn judge(
        &mut self,
        kept: &[&VerifiedOp],
        past: &Ancestry,
        deps: &[usize],
        op: &VerifiedOp,
    ) -> KeyJudgement {
        if pseudonym_signed(op) {
            return KeyJudgement::Effective;
        }
        let author = op.author();
        let in_past = |j: usize| deps.iter().any(|&d| d == j || past.contains(d, j));
        let held: Vec<usize> = (self.carriers.get(author).into_iter().flatten())
            .copied()
            .filter(|&j| in_past(j))
            .collect();
        let cert = self.cert;
        let seen = &self.seen;
        let md = (self.memo.entry((author.clone(), held)))
            .or_insert_with_key(|(_, held)| {
                let carriers = held.iter().map(|&j| (kept[j], seen.get(&j).copied()));
                Rc::new(member_view(cert, op.anchor(), author, None, carriers))
            })
            .clone();
        judge_in_view(cert, &md, op)
    }

    /// Note `op`, kept at index `i`, as a carrier when it is one, with
    /// `seen`, what it saw of the key it retires.
    pub(super) fn note(&mut self, i: usize, op: &VerifiedOp, seen: Option<u64>) {
        if let Some(member) = carrier_of(op) {
            self.carriers.entry(member.clone()).or_default().push(i);
        }
        if let Some(seq) = seen {
            self.seen.insert(i, seq);
        }
    }
}

/// `merge_from`'s judge for a `rest` op (`resume`): it answers only where
/// the snapshot's view, `snap`, decides the op's key exactly as
/// [`Judge`] over the whole set would. An op on top has every cut op in
/// its past, so its author's view at its deps is the snapshot's entry with
/// the author's `rest` carriers in its past absorbed. An op beside the cut
/// lacks some cut ops; the snapshot entry still decides it exactly when
/// its past `reach`es the entry's latest join (a cut join it lacks has a
/// lower stamp, so its record is not the current one) and the entry holds
/// no `DeviceReadmit` (judged at the op's deps, rule 7), and then counts
/// only the endorsements one of whose carriers it reaches
/// ([`MemberDevices::carriers`]; rule 2 lets any op carry one).
/// Retirements and forks are judged on the converged set (rule 5).
/// Otherwise it answers `false`, and the full merge judges. The member record at the op's deps is the view's
/// highest-stamp join ([`MemberDevices::latest_join`]): the snapshot
/// entry's, which covers every cut join, raised by the author's `rest`
/// joins in the op's past (lead decision K4b Q4).
pub(super) struct RestJudge<'s, 'a> {
    pub cert: &'s GenesisCert,
    pub snap: &'s DeviceView,
    /// `rest`'s ops kept so far and their causal pasts.
    pub ops: &'s [&'a VerifiedOp],
    pub past: &'s Ancestry,
}

/// Judge `op` in `md`, its author's view at its deps, for the member record
/// current there (lead decision K4b Q4): the view's highest-stamp
/// `MemberJoined`, else the cert's record for the creator. Both judges
/// decide through this one function.
fn judge_in_view(cert: &GenesisCert, md: &MemberDevices, op: &VerifiedOp) -> KeyJudgement {
    let record = (md.latest_join.as_ref())
        .map(|(_, record, _)| record.clone())
        .or_else(|| {
            (*op.author() == cert.creator_pseudonym).then(|| cert.creator_member_record.clone())
        });
    judge_op_at(op.anchor(), md, record.as_deref(), op.op())
}

impl RestJudge<'_, '_> {
    /// Whether `op` (its `rest` deps `in_rest`; beside the cut, the cut
    /// seqs it reaches `beside`) is decided effective; `false` is
    /// `NotOnTop`.
    pub(super) fn effective(
        &self,
        in_rest: &[usize],
        op: &VerifiedOp,
        beside: Option<&Reach>,
    ) -> bool {
        if pseudonym_signed(op) {
            return true;
        }
        let author = op.author();
        let seen;
        let entry = match (beside, self.snap.member(author)) {
            (Some(reach), Some(md)) => {
                let join_seen = match &md.latest_join {
                    Some((_, _, Some((key, seq)))) => reaches(reach, key, *seq),
                    _ => true,
                };
                if !join_seen || !md.readmits.is_empty() {
                    return false;
                }
                seen = seen_by(md, reach);
                Some(&seen)
            }
            (_, entry) => entry,
        };
        let in_past = |j: usize| in_rest.iter().any(|&d| d == j || self.past.contains(d, j));
        let carriers: Vec<usize> = (0..self.ops.len())
            .filter(|&j| carrier_of(self.ops[j]) == Some(author) && in_past(j))
            .collect();
        // `rest` holds no device op (`NotOnTop::DeviceOp`), so no
        // `DeviceRetired` carrier and nothing seen.
        let held = carriers.iter().map(|&j| (self.ops[j], None));
        let md = member_view(self.cert, op.anchor(), author, entry, held);
        matches!(judge_in_view(self.cert, &md, op), KeyJudgement::Effective)
    }
}

/// Whether the snapshot's retirements, `snap`, leave the `rest` op `op`
/// undecided (rule 5 on the converged set, lead decision K4b Q2): its
/// author retired its key at or below its seq, or the cut holds a member's
/// ops on a key it retired, above the retirement (`cut_top(p, key)`, the
/// highest seq of `p`'s cut ops on `key`), which `op` may witness, even
/// through a `DeviceRetired` carrier, lifting the floor over ops the
/// snapshot quarantined. A retirement voids only its member's ops of the
/// key (`resolve::retire_on_converged_set`). The snapshot keeps no
/// witness, so the full merge judges (`NotOnTop::DeviceJudged`).
pub(super) fn retirement_open(
    snap: &DeviceView,
    cut_top: impl Fn(&PseudonymKey, &[u8; 32]) -> Option<u64>,
    op: &VerifiedOp,
) -> bool {
    let own = &op.op().device_key;
    let retired = |md: &MemberDevices, key: &[u8; 32], seq: u64| {
        md.retired_from(key).is_some_and(|from| seq >= from)
    };
    let own_retired = (snap.member(op.author())).is_some_and(|md| retired(md, own, op.op().seq));
    own_retired
        || (snap.members.iter()).any(|(p, md)| {
            (md.retirements.keys()).any(|(k, _, _)| {
                (p, k) != (op.author(), own) && cut_top(p, k).is_some_and(|top| retired(md, k, top))
            })
        })
}

/// `md` as an op beside the cut that reaches `reach` holds it: only the
/// endorsements one of whose carriers is in its past (the cert's always).
fn seen_by(md: &MemberDevices, reach: &Reach) -> MemberDevices {
    let mut seen = md.clone();
    seen.endorsements.retain(|k, _| {
        (md.carriers.get(k)).is_some_and(|at| {
            (at.iter()).any(|(carrier, seq)| carrier.is_none_or(|key| reaches(reach, &key, *seq)))
        })
    });
    seen.carriers
        .retain(|k, _| seen.endorsements.contains_key(k));
    seen
}

#[cfg(test)]
#[path = "judge_tests.rs"]
mod tests;
