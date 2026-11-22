//! Judging an op's device key at its causal deps (ADR 0022 rules 2, 6, 7
//! and 10, plan E3.2 S7d-keys.2).
//!
//! A [`DeviceView`] is the device-key part of an op set: per member, the
//! endorsements, retirements, fork evidence and re-admissions it holds. It
//! is a grow-only set merged by union (rule 2), so the view at an op's deps
//! is the union of its deps' views, and [`judge_op`] is a pure function of
//! that view and the op (D15 "pure effects"): every reader holding the same
//! causal past judges the same.
//!
//! - An endorsement is **pending** until the reader holds every element of
//!   the set it extends (rule 2).
//! - An op of a key with no endorsement in its causal past (nor carried by
//!   the op at seq 0) is **pending**, never refused (rule 6).
//! - A key is **effective** when it is among the member's first
//!   [`MAX_EFFECTIVE_DEVICE_KEYS`] live endorsements by `(incarnation, key
//!   bytes)`; an endorsement whose own extended set already holds that many
//!   live keys is inert in every carrier (rule 6).
//! - An endorsement answering a fork is effective only with a
//!   `DeviceReadmit` naming the same key, incarnation and fork evidence
//!   (rules 7 and 10).
//! - An op is judged by the key that signed it, never by a device op's
//!   subject, so a `DeviceReadmit` survives its subject's retirement (rule 6).

use std::collections::{BTreeMap, BTreeSet};

use rekindle_types::governance::{
    DeviceEndorsement, DeviceSetElement, DeviceSetElementKind, GenesisCert, GovernanceEntry,
    GovernanceOp,
};
use rekindle_types::id::PseudonymKey;

use crate::device::{endorsement_holds, retirement_holds};
use crate::op::VerifiedOp;
use crate::state::GovernanceState;

/// Most effective device keys per member, the first included (ADR 0022
/// rule 6, Signal-Server `MAX_DEVICES`).
pub const MAX_EFFECTIVE_DEVICE_KEYS: usize = 6;

/// One member's device-key elements in a view.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MemberDevices {
    /// `M(p)`, from the member's `MemberJoined`: the context its
    /// endorsements and retirements are signed in (rule 10).
    pub member_record: Option<String>,
    /// Valid endorsements by `(incarnation, device key)`, rule 6's order.
    pub endorsements: BTreeMap<(u64, [u8; 32]), DeviceEndorsement>,
    /// Valid retirements by `(device key, incarnation)`: the lowest
    /// `lastSeq` signed for it (rule 5).
    pub retirements: BTreeMap<([u8; 32], u64), u64>,
    /// Fork evidence by device key: the lowest seq it was forked at (rule 5).
    pub forks: BTreeMap<[u8; 32], u64>,
    /// `DeviceReadmit`s naming this member: `(new key, incarnation, fork
    /// evidence)` (rule 7).
    pub readmits: BTreeSet<([u8; 32], u64, [u8; 32])>,
}

impl MemberDevices {
    /// The element ids this member's set holds (rule 10).
    #[must_use]
    pub fn elements(&self) -> BTreeSet<DeviceSetElement> {
        let endorsed = (self.endorsements.keys())
            .map(|(inc, key)| DeviceSetElement::new(DeviceSetElementKind::Endorsement, *key, *inc));
        let retired = (self.retirements.keys())
            .map(|(key, inc)| DeviceSetElement::new(DeviceSetElementKind::Retirement, *key, *inc));
        endorsed.chain(retired).collect()
    }

    /// The member records `p`'s endorsements may be signed for (rule 10):
    /// the first `M(p)` and every record a first device's endorsement binds
    /// in its own context (a re-join's new `M(p)`, lead decision 63),
    /// derived from the set alone so the view stays a grow-only union.
    #[must_use]
    pub fn member_records(&self, p: &PseudonymKey, anchor: &str) -> BTreeSet<String> {
        let firsts = (self.endorsements.values())
            .filter(|e| endorsement_holds(p, anchor, &e.device_record, e))
            .map(|e| e.device_record.clone());
        self.member_record.iter().cloned().chain(firsts).collect()
    }

    /// `p`'s endorsement of `key` signed for member record `record` (rules
    /// 1 and 10), the lowest incarnation's: `None` when no endorsement of
    /// the key binds that record, so a key endorsed only under an ended
    /// membership is unendorsed for the current one (lead decision 63).
    #[must_use]
    pub fn endorsement_for(
        &self,
        p: &PseudonymKey,
        anchor: &str,
        record: &str,
        key: &[u8; 32],
    ) -> Option<&DeviceEndorsement> {
        (self.endorsements.values())
            .find(|e| e.device_key == *key && endorsement_holds(p, anchor, record, e))
    }

    /// The `M(p,d)` that `key`'s op chain lives on when `key` is a further
    /// device of `p` (ADR 0022 rule 3, "every further device record is
    /// bound by its `DeviceKey` endorsement (`deviceRecord`, ...)"): the
    /// record an endorsement of `key` binds while signed in the context of
    /// a member record, not of the record it binds (rule 10). `None` for a
    /// first device, whose endorsement is signed for the record it binds
    /// ("The first device's record is `M(p)`, bound by `MemberJoined`"),
    /// and for a key with no endorsement here.
    #[must_use]
    pub fn device_record(&self, p: &PseudonymKey, anchor: &str, key: &[u8; 32]) -> Option<&str> {
        (self.endorsements.values())
            .find(|e| e.device_key == *key && !endorsement_holds(p, anchor, &e.device_record, e))
            .map(|e| e.device_record.as_str())
    }

    /// The lowest seq above which `key` signs nothing valid: its lowest
    /// retirement `lastSeq` + 1 or its fork seq, whichever is lower.
    fn void_from(&self, key: &[u8; 32]) -> Option<u64> {
        let retired = (self.retirements.iter())
            .filter(|((k, _), _)| k == key)
            .map(|(_, last)| last.saturating_add(1))
            .min();
        retired
            .into_iter()
            .chain(self.forks.get(key).copied())
            .min()
    }
}

/// The device-key part of an op set (module doc).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DeviceView {
    pub(crate) members: BTreeMap<PseudonymKey, MemberDevices>,
}

impl DeviceView {
    /// `member`'s elements, if any.
    #[must_use]
    pub fn member(&self, member: &PseudonymKey) -> Option<&MemberDevices> {
        self.members.get(member)
    }

    /// The union of two views (rule 2: a grow-only set).
    pub fn union(&mut self, other: &Self) {
        for (member, theirs) in &other.members {
            let ours = self.members.entry(member.clone()).or_default();
            if ours.member_record.is_none() {
                ours.member_record.clone_from(&theirs.member_record);
            }
            for (k, e) in &theirs.endorsements {
                ours.endorsements.entry(*k).or_insert_with(|| e.clone());
            }
            for (k, last) in &theirs.retirements {
                let mine = ours.retirements.entry(*k).or_insert(*last);
                *mine = (*mine).min(*last);
            }
            for (k, seq) in &theirs.forks {
                let mine = ours.forks.entry(*k).or_insert(*seq);
                *mine = (*mine).min(*seq);
            }
            ours.readmits.extend(theirs.readmits.iter().copied());
        }
    }

    /// Seed the creator's first device from `cert` of community `anchor`
    /// (plan E3.2 S7d-keys.5): the creator joins by the cert, not by a
    /// `MemberJoined`, so its endorsement is `creator_device`, signed in the
    /// context of `creator_member_record` (rule 10).
    pub fn seed_genesis(&mut self, anchor: &str, cert: &GenesisCert) {
        // A cert without a creator device adds nothing, so the view (and
        // the state root that covers it) is the op set's alone.
        let Some(e) = &cert.creator_device else {
            return;
        };
        let p = &cert.creator_pseudonym;
        let md = self.members.entry(p.clone()).or_default();
        md.member_record
            .get_or_insert_with(|| cert.creator_member_record.clone());
        take_endorsement(md, p, anchor, e);
    }

    /// Record fork evidence for `member`'s `key` at `seq` (rule 5).
    pub fn fork(&mut self, member: &PseudonymKey, key: [u8; 32], seq: u64) {
        let forks = &mut self.members.entry(member.clone()).or_default().forks;
        let at = forks.entry(key).or_insert(seq);
        *at = (*at).min(seq);
    }

    /// Harvest the device elements `op` of community `anchor` carries
    /// (rule 2: from any op that carries them). An element whose `p`
    /// signature does not hold in its context is not taken. An op carrying
    /// none leaves the view as it was.
    pub fn absorb(&mut self, anchor: &str, op: &GovernanceOp) {
        let p = &op.author;
        match &op.body {
            GovernanceEntry::MemberJoined {
                member_record,
                first_device,
                ..
            } => {
                let md = self.members.entry(p.clone()).or_default();
                md.member_record
                    .get_or_insert_with(|| member_record.clone());
                // A first device's endorsement is signed for the record its
                // own `MemberJoined` names (rules 3 and 10), a re-join's new
                // `M(p)` included (lead decision 63).
                if let Some(e) = first_device {
                    if endorsement_holds(p, anchor, member_record, e) {
                        (md.endorsements)
                            .entry((e.incarnation, e.device_key))
                            .or_insert_with(|| e.clone());
                    }
                }
            }
            GovernanceEntry::DeviceKey(e) => {
                take_endorsement(self.members.entry(p.clone()).or_default(), p, anchor, e);
            }
            GovernanceEntry::DeviceRetired(r) => {
                let md = self.members.entry(p.clone()).or_default();
                let held = (md.member_record.as_deref())
                    .is_some_and(|record| retirement_holds(p, anchor, record, r));
                if held {
                    let last = md
                        .retirements
                        .entry((r.device_key, r.incarnation))
                        .or_insert(r.last_seq);
                    *last = (*last).min(r.last_seq);
                }
            }
            GovernanceEntry::DeviceReadmit(r) => {
                let subject = self.members.entry(r.member.clone()).or_default();
                (subject.readmits).insert((r.new_key, r.incarnation, r.fork_evidence));
            }
            _ => {}
        }
    }

    /// [`Self::absorb`] for a verified op, in the anchor it was verified
    /// for: how a merge harvests the view its state carries (plan E3.2
    /// K-CP).
    pub fn harvest(&mut self, op: &VerifiedOp) {
        self.absorb(op.anchor(), op.op());
    }
}

impl GovernanceState {
    /// The record the op `device_key` signed as `author` in community
    /// `anchor` lives on, the one locator every reference to the op names
    /// (ADR 0022 rule 3: "Each device key has its own `M(p,d)`", subkey 2
    /// `GovHead` is "that device's own op chain"): a further device's
    /// `M(p,d)` from the device set ([`MemberDevices::device_record`]),
    /// else the author's member record ([`Self::author_record`]: the first
    /// device's `M(p)`, the cert's for the creator).
    #[must_use]
    pub fn op_record(
        &self,
        cert: &GenesisCert,
        anchor: &str,
        author: &PseudonymKey,
        device_key: &[u8; 32],
    ) -> Option<String> {
        let further = (self.devices.member(author))
            .and_then(|md| md.device_record(author, anchor, device_key));
        further
            .map(str::to_owned)
            .or_else(|| self.author_record(cert, author))
    }
}

fn take_endorsement(md: &mut MemberDevices, p: &PseudonymKey, anchor: &str, e: &DeviceEndorsement) {
    let held = (md.member_records(p, anchor).iter()).any(|r| endorsement_holds(p, anchor, r, e));
    if held {
        (md.endorsements)
            .entry((e.incarnation, e.device_key))
            .or_insert_with(|| e.clone());
    }
}

/// Why an op's key is not judged yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyPending {
    /// No endorsement of the key in the op's causal past (rule 6).
    Unendorsed,
    /// The key's endorsement extends a set element the reader lacks (rule 2).
    ExtendedSet(DeviceSetElement),
}

/// Why an op's key is not effective at its deps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyRefusal {
    /// Retired, or forked, at or below the op's seq (rules 4, 5).
    Retired,
    /// Its endorsement's extended set already held six live keys (rule 6).
    Inert,
    /// Not among the member's first six live endorsements (rule 6).
    OverCap,
    /// It answers a fork with no matching `DeviceReadmit` (rules 7, 10).
    NeedsReadmit,
    /// Its `deviceRecord` is bound by an earlier `(incarnation, key)`
    /// endorsement of another key: one record per device (rule 3).
    RecordTaken,
}

/// An op's device key judged at its causal deps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyJudgement {
    Effective,
    Pending(KeyPending),
    Refused(KeyRefusal),
}

/// Judge `op`'s device key at `at_deps`, the view of its causal past. The
/// key is the op's signer's; a device op's subject is never consulted.
#[must_use]
pub fn judge_op(anchor: &str, at_deps: &DeviceView, op: &GovernanceOp) -> KeyJudgement {
    let mut view = at_deps.clone();
    // An endorsement the op carries at seq 0 counts for the op itself.
    let carries = matches!(
        &op.body,
        GovernanceEntry::MemberJoined { first_device: Some(e), .. } | GovernanceEntry::DeviceKey(e)
            if e.device_key == op.device_key
    );
    if op.seq == 0 && carries {
        view.absorb(anchor, op);
    }
    let Some(md) = view.member(&op.author) else {
        return KeyJudgement::Pending(KeyPending::Unendorsed);
    };
    judge_key(md, &op.device_key, op.seq)
}

/// Judge `key` of a member whose elements at the op's deps are `md`, for
/// an op at `seq`.
#[must_use]
pub fn judge_key(md: &MemberDevices, key: &[u8; 32], seq: u64) -> KeyJudgement {
    let Some(((incarnation, _), e)) = md.endorsements.iter().find(|((_, k), _)| k == key) else {
        return KeyJudgement::Pending(KeyPending::Unendorsed);
    };
    let held = md.elements();
    if let Some(missing) = e.extends_elements.iter().find(|x| !held.contains(x)) {
        return KeyJudgement::Pending(KeyPending::ExtendedSet(*missing));
    }
    if md.void_from(key).is_some_and(|from| seq >= from) {
        return KeyJudgement::Refused(KeyRefusal::Retired);
    }
    if inert(e) {
        return KeyJudgement::Refused(KeyRefusal::Inert);
    }
    if record_taken(md, (*incarnation, *key), &e.device_record) {
        return KeyJudgement::Refused(KeyRefusal::RecordTaken);
    }
    let live = (md.endorsements.iter())
        .filter(|((_, k), e)| md.void_from(k).is_none() && !inert(e))
        .map(|((inc, k), _)| (*inc, *k))
        .take(MAX_EFFECTIVE_DEVICE_KEYS)
        .any(|(inc, k)| (inc, k) == (*incarnation, *key));
    // A key retired above this op's seq still signed it while live.
    if !live && md.void_from(key).is_none() {
        return KeyJudgement::Refused(KeyRefusal::OverCap);
    }
    if e.answers_fork()
        && !md
            .readmits
            .contains(&(*key, e.incarnation, e.fork_evidence))
    {
        return KeyJudgement::Refused(KeyRefusal::NeedsReadmit);
    }
    KeyJudgement::Effective
}

/// Whether an endorsement of another key, earlier in rule 6's
/// `(incarnation, key)` order than `at`, binds `record` (rule 3: each key
/// has its own `M(p,d)`, and the first device's is `M(p)`, so a later key
/// naming `M(p)` or a sibling's record is refused). A pure function of the
/// set, so every reader holding it judges the same, in any arrival order.
fn record_taken(md: &MemberDevices, at: (u64, [u8; 32]), record: &str) -> bool {
    (md.endorsements.range(..at)).any(|((_, k), other)| *k != at.1 && other.device_record == record)
}

/// Whether `e`'s own extended set already holds six live keys (rule 6).
fn inert(e: &DeviceEndorsement) -> bool {
    let retired: BTreeSet<[u8; 32]> = (e.extends_elements.iter())
        .filter(|x| x.kind == DeviceSetElementKind::Retirement)
        .map(|x| x.device_key)
        .collect();
    let live: BTreeSet<[u8; 32]> = (e.extends_elements.iter())
        .filter(|x| x.kind == DeviceSetElementKind::Endorsement && !retired.contains(&x.device_key))
        .map(|x| x.device_key)
        .collect();
    live.len() >= MAX_EFFECTIVE_DEVICE_KEYS
}

#[cfg(test)]
#[path = "device_judge_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "device_enrol_tests.rs"]
mod enrol_tests;

#[cfg(test)]
#[path = "device_rejoin_tests.rs"]
mod rejoin_tests;
