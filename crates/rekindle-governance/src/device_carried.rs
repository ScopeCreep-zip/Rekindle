//! The device elements one op carries and how a member's view takes them
//! (ADR 0022 rules 2, 4, 5 and 10; plan E3.2 S7d-keys.2 and .10): an
//! endorsement from any carrier, a retirement at its carrier floor, and a
//! `DeviceReEnrol`'s pair only on the new key's seq 0 (K4b.4 R3).

use rekindle_types::governance::{
    DeviceEndorsement, DeviceReEnrol, DeviceRetired, GovernanceEntry, GovernanceOp,
};
use rekindle_types::id::PseudonymKey;

use crate::device::{endorsement_holds, retirement_holds};
use crate::device_judge::MemberDevices;

/// The `DeviceReEnrol` `op` carries: on the new key's seq 0 only (ADR 0022
/// rule 4, K4b.4 R3, plan E3.2 S7d-keys.10); anywhere else it carries no
/// device element.
#[must_use]
pub fn re_enrol_of(op: &GovernanceOp) -> Option<&DeviceReEnrol> {
    match &op.body {
        GovernanceEntry::DeviceReEnrol(r) if r.carried_by(&op.device_key, op.seq) => Some(r),
        _ => None,
    }
}

/// The `DeviceRetired` `op` carries: its own arm's, or a re-enrolment's
/// ([`re_enrol_of`]).
#[must_use]
pub fn retirement_of(op: &GovernanceOp) -> Option<&DeviceRetired> {
    match &op.body {
        GovernanceEntry::DeviceRetired(r) => Some(r),
        _ => re_enrol_of(op).map(|re| &re.retired),
    }
}

/// Take `r` into `md` when `p`'s signature holds for the member record,
/// at its carrier floor: the highest of its signed `lastSeq` and `seen`
/// (rule 5), a pure max per entry.
pub(crate) fn take_retirement(
    md: &mut MemberDevices,
    p: &PseudonymKey,
    anchor: &str,
    r: &DeviceRetired,
    seen: Option<u64>,
) {
    let held =
        (md.member_record.as_deref()).is_some_and(|record| retirement_holds(p, anchor, record, r));
    if held {
        let floor = r.last_seq.max(seen.unwrap_or(0));
        let at = md
            .retirements
            .entry((r.device_key, r.incarnation, r.last_seq))
            .or_insert(floor);
        *at = (*at).max(floor);
    }
}

/// Take `e` into `md` when its signature holds for one of the member's
/// records, noting its carrier `at` (`None`: the cert).
pub(crate) fn take_endorsement(
    md: &mut MemberDevices,
    p: &PseudonymKey,
    anchor: &str,
    e: &DeviceEndorsement,
    at: Option<([u8; 32], u64)>,
) {
    let held = (md.member_records(p, anchor).iter()).any(|r| endorsement_holds(p, anchor, r, e));
    if held {
        let k = (e.incarnation, e.device_key);
        (md.endorsements).entry(k).or_insert_with(|| e.clone());
        let (carrier, seq) = at.map_or((None, 0), |(key, seq)| (Some(key), seq));
        note_carrier(md, k, carrier, seq);
    }
}

/// Lower the endorsement `k`'s carrier `carrier` to `seq` (a pure min, so
/// the view stays an order-free union).
pub(crate) fn note_carrier(
    md: &mut MemberDevices,
    k: (u64, [u8; 32]),
    carrier: Option<[u8; 32]>,
    seq: u64,
) {
    let at = md
        .carriers
        .entry(k)
        .or_default()
        .entry(carrier)
        .or_insert(seq);
    *at = (*at).min(seq);
}
