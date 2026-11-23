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

use std::collections::BTreeMap;

use rekindle_types::governance::{DeviceEndorsement, GenesisCert, GovernanceEntry};

use super::resolve::Ancestry;
use crate::device_judge::{DeviceView, MemberDevices};
use crate::op::VerifiedOp;

/// Per op: an own-key `DeviceReadmit` whose co-signer is not pre-fork,
/// refused and kept as ancestry (never in the device view).
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
            if r.member != *ops[i].author() {
                return false;
            }
            let none = MemberDevices::default();
            let md = devices.member(&r.member).unwrap_or(&none);
            let cosigner = ops[i].op().device_key;
            let held = |j: usize| past.contains(i, j) && *ops[j].author() == r.member;
            let paired = |j: usize| {
                let (key, seq) = (ops[j].op().device_key, ops[j].op().seq);
                fork_at.get(&key) == Some(&seq)
                    && (0..i).any(|k| {
                        k != j && held(k) && (ops[k].op().device_key, ops[k].op().seq) == (key, seq)
                    })
            };
            let events: Vec<usize> = (0..i)
                .filter(|&j| held(j) && (paired(j) || retires(md, ops[j])))
                .collect();
            let endorsed: Vec<usize> = (0..i)
                .filter(|&j| held(j) && endorses(md, ops[j], &cosigner))
                .collect();
            let in_cert = r.member == cert.creator_pseudonym
                && (cert.creator_device.as_ref()).is_some_and(|e| e.device_key == cosigner);
            !in_cert && !(events.iter()).all(|&f| endorsed.iter().any(|&c| past.contains(f, c)))
        })
        .collect()
}

/// Whether `op` carries a valid endorsement of `key` (one the view holds).
fn endorses(md: &MemberDevices, op: &VerifiedOp, key: &[u8; 32]) -> bool {
    let carried: Option<&DeviceEndorsement> = match op.body() {
        GovernanceEntry::MemberJoined {
            first_device: Some(e),
            ..
        }
        | GovernanceEntry::DeviceKey(e) => Some(e),
        _ => None,
    };
    carried.is_some_and(|e| e.device_key == *key && md.endorsements.values().any(|v| v == e))
}

/// Whether `op` carries a valid `DeviceRetired` (one the view holds).
fn retires(md: &MemberDevices, op: &VerifiedOp) -> bool {
    match op.body() {
        GovernanceEntry::DeviceRetired(r) => {
            (md.retirements).contains_key(&(r.device_key, r.incarnation, r.last_seq))
        }
        _ => false,
    }
}
