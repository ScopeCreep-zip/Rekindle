//! `merge` moderation CRDT apply rules.

use crate::op::OpStamp;

use super::{
    AdmissionDecision, AutoModRuleState, GovernanceEntry, GovernanceState, MemberState,
    PseudonymKey, TimeoutState,
};
use crate::state::ModReportState;

pub(super) fn apply_moderation(
    entry: &GovernanceEntry,
    stamp: OpStamp,
    state: &mut GovernanceState,
) {
    match entry {
        GovernanceEntry::BanEntry { target, .. } => {
            let prev = state
                .ban_stamps
                .get(target)
                .copied()
                .unwrap_or(OpStamp::MIN);
            if stamp > prev {
                state.bans.insert(target.clone());
                state.ban_stamps.insert(target.clone(), stamp);
            }
        }

        GovernanceEntry::UnbanEntry { target } => {
            let prev = state
                .ban_stamps
                .get(target)
                .copied()
                .unwrap_or(OpStamp::MIN);
            if stamp > prev {
                state.bans.remove(target);
                state.ban_stamps.insert(target.clone(), stamp);
            }
        }

        // ── Timeout ──
        GovernanceEntry::TimeoutEntry {
            target,
            duration_seconds,
            started_at,
            ..
        } => {
            let existing_stamp = state.timeouts.get(target).map_or(OpStamp::MIN, |t| t.stamp);
            if stamp > existing_stamp {
                state.timeouts.insert(
                    target.clone(),
                    TimeoutState {
                        duration_seconds: *duration_seconds,
                        started_at: *started_at,
                        stamp,
                    },
                );
            }
        }

        GovernanceEntry::RemoveTimeoutEntry { target } => {
            let existing_stamp = state.timeouts.get(target).map_or(OpStamp::MIN, |t| t.stamp);
            if stamp > existing_stamp {
                state.timeouts.remove(target);
            }
        }

        // ── Metadata: LWW ──
        GovernanceEntry::AdminDelete { message_id, .. } => {
            state.admin_deletes.insert(*message_id);
        }

        // ── Plate Gate lazy channel records (architecture §15.4):
        //    LWW per (channel_id, segment_index). First-writer wins on
        //    creation; later messages from the same segment write to the
        //    same record (no migration). ──
        GovernanceEntry::AutoModRule {
            rule_id,
            name,
            enabled,
            trigger_json,
            action,
        } => {
            let existing_stamp = state
                .automod_rules
                .get(rule_id)
                .map_or(OpStamp::MIN, |r| r.stamp);
            if stamp > existing_stamp {
                state.automod_rules.insert(
                    *rule_id,
                    AutoModRuleState {
                        name: name.clone(),
                        enabled: *enabled,
                        trigger_json: trigger_json.clone(),
                        action: *action,
                        stamp,
                    },
                );
            }
        }

        // ── Role archived (tombstone) ──
        _ => unreachable!("apply_moderation: unexpected variant"),
    }
}

/// Admission CRDT: pending requests and the decisions that clear them.
///
/// A `MemberJoined` under approval waits in `pending_members`; the two decision variants
/// are an LWW-Flag per target that removes the pending entry. A decision
/// is idempotent and order-independent — replaying it, or seeing it
/// before the request it answers, converges to the same state, which is
/// what the merge property tests require.
pub(super) fn apply_admission(
    entry: &GovernanceEntry,
    stamp: OpStamp,
    state: &mut GovernanceState,
) {
    match entry {
        GovernanceEntry::MemberApproved { target } => {
            record_decision(state, target, true, stamp);
        }
        GovernanceEntry::MemberRejected { target, .. } => {
            record_decision(state, target, false, stamp);
        }
        _ => {}
    }
}

/// LWW-Flag per target: highest lamport wins, ties broken by leaving the
/// existing decision in place (the merge already sorts by
/// `(lamport, author)`, so the last writer at equal lamport is
/// deterministic across peers).
fn record_decision(
    state: &mut GovernanceState,
    target: &PseudonymKey,
    approved: bool,
    stamp: OpStamp,
) {
    let supersedes = state
        .admitted
        .get(target)
        .is_none_or(|existing| stamp >= existing.stamp);
    if supersedes {
        state
            .admitted
            .insert(target.clone(), AdmissionDecision { approved, stamp });
        if let Some(pending) = state.pending_members.remove(target) {
            if approved {
                state.members.insert(
                    target.clone(),
                    MemberState {
                        member_record: pending.member_record,
                        member_record_owner: pending.member_record_owner,
                        joined: pending.stamp,
                    },
                );
            }
        }
    }
}

/// `ModReport`: a G-Set per report id. Two reports under one id keep the
/// lowest `(lamport, reporter)`, so every peer's queue is the same.
pub(super) fn apply_mod_report(
    author: &PseudonymKey,
    entry: &GovernanceEntry,
    stamp: OpStamp,
    state: &mut GovernanceState,
) {
    let GovernanceEntry::ModReport {
        report_id,
        target,
        reason,
    } = entry
    else {
        return;
    };
    let incoming = ModReportState {
        reporter: author.clone(),
        target: target.clone(),
        reason: reason.clone(),
        stamp,
    };
    let slot = state
        .mod_reports
        .entry(*report_id)
        .or_insert_with(|| incoming.clone());
    if (incoming.stamp, &incoming.reporter.0) < (slot.stamp, &slot.reporter.0) {
        *slot = incoming;
    }
}
