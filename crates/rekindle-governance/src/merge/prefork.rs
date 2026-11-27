//! The own-key `DeviceReadmit`'s pre-fork co-signer (ADR 0022 rule 7, plan
//! E3.2 K4b Q7): a member's own readmit counts only when the key that
//! signed it was endorsed in the **strict** causal past of **every** fork
//! event of that member the readmit's deps hold, so approvals never chain.
//!
//! A fork event is each op of a fork pair (two ops of one key at its fork
//! seq, both held: the evidence) and each `DeviceRetired` carrier. The
//! intersection, not the lowest stamp: an endorsement after one op of a
//! pair and concurrent with the other is not before the fork. The
//! replacement a `DeviceRetired` carrier carries is in no strict past of
//! that carrier, so it is never pre-fork. A pure function of the op set.
//!
//! The one exception is the exempt replacement (ADR 0022 rules 4 and 7,
//! K4b.4 R3, plan E3.2 S7d-keys.10): the key a `DeviceReEnrol` carries
//! when its strict past holds no fork event of its member, of concurrent
//! ones the lowest `(lamport, op_hash)` ([`exempt_replacement`]).
//!
//! Every `DeviceReadmit`, own-key or a moderator's, answers one fork event
//! of its member held at its deps (rule 7, amended 2026-11-23): its
//! `forkEvidence` is a fork pair's hash or a retirement's
//! ([`fork_evidence_of`]); any other is refused.

use std::collections::BTreeMap;

use rekindle_types::governance::{DeviceEndorsement, GenesisCert, GovernanceEntry, OpHash};
use rekindle_types::id::PseudonymKey;

use super::resolve::Ancestry;
use crate::device::{fork_evidence_hash, retirement_evidence_hash};
use crate::device_judge::{re_enrol_of, retirement_of, DeviceView, MemberDevices};
use crate::op::VerifiedOp;

/// Per forked device key, its fork seq: the lowest seq at which `sorted`
/// (stamp order) holds two different ops of that key (rule 5).
pub(super) fn fork_seqs(sorted: &[&VerifiedOp]) -> BTreeMap<[u8; 32], u64> {
    // Sorted by (seq, device key, hash), two ops at one (key, seq) with
    // different hashes sit next to each other; seq leads the key because
    // it is a cheap compare that settles most pairs.
    let mut by_seq: Vec<(u64, &[u8; 32], OpHash)> = sorted
        .iter()
        .map(|op| (op.op().seq, &op.op().device_key, op.hash()))
        .collect();
    by_seq.sort_unstable();
    let mut fork_at: BTreeMap<[u8; 32], u64> = BTreeMap::new();
    for pair in by_seq.windows(2) {
        let ((seq, key, a), (next_seq, next_key, b)) = (pair[0], pair[1]);
        if key == next_key && seq == next_seq && a != b {
            let slot = fork_at.entry(*key).or_insert(u64::MAX);
            *slot = (*slot).min(seq);
        }
    }
    fork_at
}

/// Per op: a `DeviceReadmit` naming no fork event of its member held at
/// its deps (every readmit, rule 7), or an own-key readmit whose co-signer
/// is not pre-fork, refused and kept as ancestry (never in the device
/// view).
pub(super) fn own_readmits_refused(
    cert: &GenesisCert,
    ops: &[&VerifiedOp],
    past: &Ancestry,
    fork_at: &BTreeMap<[u8; 32], u64>,
    devices: &DeviceView,
) -> Vec<bool> {
    (0..ops.len())
        .map(|i| {
            let GovernanceEntry::DeviceReadmit(r) = ops[i].body() else {
                return false;
            };
            let none = MemberDevices::default();
            let md = devices.member(&r.member).unwrap_or(&none);
            let held = |j: usize| past.contains(i, j) && *ops[j].author() == r.member;
            let events = fork_events(ops, i, &held, fork_at, md);
            let record = &r.member_record;
            let answered = fork_evidence_of(ops, i, &held, md, &events, record);
            if !answered.contains(&r.fork_evidence) {
                return true;
            }
            if r.member != *ops[i].author() {
                return false;
            }
            let cosigner = ops[i].op().device_key;
            let endorsed = endorsers(ops, i, &held, md, |e| e.device_key == cosigner);
            let in_cert = r.member == cert.creator_pseudonym
                && (cert.creator_device.as_ref()).is_some_and(|e| e.device_key == cosigner);
            after_fork(past, &events, &endorsed, in_cert)
        })
        .collect()
}

/// The member's fork events among `ops[..upto]` that `held` (in the
/// judged op's past and the member's): each op of a fork pair whose
/// sibling is held too, and each valid `DeviceRetired` carrier.
pub(super) fn fork_events(
    ops: &[&VerifiedOp],
    upto: usize,
    held: &impl Fn(usize) -> bool,
    fork_at: &BTreeMap<[u8; 32], u64>,
    md: &MemberDevices,
) -> Vec<usize> {
    let paired = |j: usize| {
        let (key, seq) = (ops[j].op().device_key, ops[j].op().seq);
        fork_at.get(&key) == Some(&seq)
            && (0..upto).any(|k| {
                k != j && held(k) && (ops[k].op().device_key, ops[k].op().seq) == (key, seq)
            })
    };
    (0..upto)
        .filter(|&j| held(j) && (paired(j) || retires(md, ops[j])))
        .collect()
}

/// The evidence hashes of the fork events `events` (indices into
/// `ops[..upto]`, of one member, its record `record`): each op of a fork
/// pair names the pair's hash (rule 5) and each `DeviceRetired` carrier
/// its retirement's ([`retirement_evidence_hash`]), in `events`' order.
pub(super) fn fork_evidence_of(
    ops: &[&VerifiedOp],
    upto: usize,
    held: &impl Fn(usize) -> bool,
    md: &MemberDevices,
    events: &[usize],
    record: &str,
) -> Vec<[u8; 32]> {
    let mut out = Vec::new();
    for &j in events {
        let (key, seq) = (ops[j].op().device_key, ops[j].op().seq);
        let anchor = ops[j].anchor();
        let sibling = (0..upto).find(|&k| {
            k != j
                && held(k)
                && (ops[k].op().device_key, ops[k].op().seq) == (key, seq)
                && ops[k].hash() != ops[j].hash()
        });
        if let Some(k) = sibling {
            out.push(fork_evidence_hash(
                anchor,
                &key,
                seq,
                [ops[j].hash(), ops[k].hash()],
            ));
        }
        if let Some(r) = retirement_of(ops[j].op()) {
            if retires(md, ops[j]) {
                out.push(retirement_evidence_hash(anchor, record, r));
            }
        }
    }
    out
}

/// The evidence a post-fork endorsement of `member` names when built on
/// `ops` (ADR 0022 rule 7, amended 2026-11-23): its lowest-stamp fork
/// event's ([`fork_evidence_of`]), with `md` its device set there and
/// `record` its member record; `None` when `ops` hold no fork event of it.
#[must_use]
pub fn first_fork_evidence(
    ops: &[VerifiedOp],
    member: &PseudonymKey,
    md: &MemberDevices,
    record: &str,
) -> Option<[u8; 32]> {
    let mut sorted: Vec<&VerifiedOp> = ops.iter().collect();
    sorted.sort_by_key(|op| op.stamp());
    sorted.dedup_by_key(|op| op.hash());
    let fork_at = fork_seqs(&sorted);
    let held = |j: usize| sorted[j].author() == member;
    let events = fork_events(&sorted, sorted.len(), &held, &fork_at, md);
    let first = events.first().copied()?;
    fork_evidence_of(&sorted, sorted.len(), &held, md, &[first], record)
        .first()
        .copied()
}

/// The ops among `ops[..upto]` that `held` and carry a valid endorsement
/// `which` picks.
pub(super) fn endorsers(
    ops: &[&VerifiedOp],
    upto: usize,
    held: &impl Fn(usize) -> bool,
    md: &MemberDevices,
    which: impl Fn(&DeviceEndorsement) -> bool,
) -> Vec<usize> {
    (0..upto)
        .filter(|&j| held(j) && endorses(md, ops[j], &which))
        .collect()
}

/// Whether an endorsement carried by `carriers` (`in_cert`: by the cert,
/// in every op's past) is not before the fork: some fork event in `events`
/// has none of its carriers in its strict past (the intersection, K4b Q7).
/// The own-key readmit's co-signer must be before it; a key endorsed after
/// it needs a readmit (rule 7, plan E3.2 K4b.4).
pub(super) fn after_fork(
    past: &Ancestry,
    events: &[usize],
    carriers: &[usize],
    in_cert: bool,
) -> bool {
    !in_cert && !(events.iter()).all(|&f| carriers.iter().any(|&c| past.contains(f, c)))
}

/// Whether `op` carries a valid endorsement (one the view holds) `which`
/// picks.
fn endorses(
    md: &MemberDevices,
    op: &VerifiedOp,
    which: &impl Fn(&DeviceEndorsement) -> bool,
) -> bool {
    let carried: Option<&DeviceEndorsement> = match op.body() {
        GovernanceEntry::MemberJoined {
            first_device: Some(e),
            ..
        }
        | GovernanceEntry::DeviceKey(e) => Some(e),
        _ => re_enrol_of(op.op()).map(|r| &r.endorsement),
    };
    carried.is_some_and(|e| which(e) && md.endorsements.values().any(|v| v == e))
}

/// Whether `op` carries a valid `DeviceRetired` (one the view holds), its
/// own arm's or a `DeviceReEnrol`'s ([`retirement_of`]).
fn retires(md: &MemberDevices, op: &VerifiedOp) -> bool {
    retirement_of(op.op())
        .is_some_and(|r| (md.retirements).contains_key(&(r.device_key, r.incarnation, r.last_seq)))
}

/// ADR 0022 rule 7's exempt replacement (rules 4 and 7 as amended in K4b.4
/// R3, plan E3.2 S7d-keys.10): whether the endorsement `at` (incarnation,
/// key) is the one the member's exempt `DeviceReEnrol` carries among the
/// ops that `held` (in the judged op's past and the member's). Exempt is a
/// valid re-enrolment whose strict past holds none of the member's fork
/// events `events`; of several such (concurrent ones: one in another's past
/// has it as a fork event) only the lowest `(lamport, op_hash)` is. A pure
/// function of the judged op's past, so ops already judged stay judged
/// (rule 6) and a later op whose deps hold both copies' re-enrolments
/// admits only the lowest one's key.
pub(super) fn exempt_replacement(
    ops: &[&VerifiedOp],
    held: &impl Fn(usize) -> bool,
    past: &Ancestry,
    events: &[usize],
    md: &MemberDevices,
    at: (u64, [u8; 32]),
) -> bool {
    (0..ops.len())
        .filter(|&j| held(j) && re_enrol_of(ops[j].op()).is_some() && retires(md, ops[j]))
        .filter(|&j| !events.iter().any(|&f| past.contains(j, f)))
        .min_by_key(|&j| ops[j].stamp())
        .and_then(|j| re_enrol_of(ops[j].op()))
        .is_some_and(|r| {
            let e = &r.endorsement;
            (e.incarnation, e.device_key) == at && md.endorsements.get(&at) == Some(e)
        })
}
