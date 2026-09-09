//! `merge` moderation CRDT apply rules.

use super::{
    AdmissionDecision, AutoModRuleState, GovernanceEntry, GovernanceState, PendingMemberState,
    PseudonymKey, TimeoutState,
};

pub(super) fn apply_moderation(entry: &GovernanceEntry, state: &mut GovernanceState) {
    match entry {
        GovernanceEntry::BanEntry {
            target, lamport, ..
        } => {
            let prev = state.ban_lamports.get(target).copied().unwrap_or(0);
            if *lamport > prev {
                state.bans.insert(target.clone());
                state.ban_lamports.insert(target.clone(), *lamport);
            }
        }

        GovernanceEntry::UnbanEntry { target, lamport } => {
            let prev = state.ban_lamports.get(target).copied().unwrap_or(0);
            if *lamport > prev {
                state.bans.remove(target);
                state.ban_lamports.insert(target.clone(), *lamport);
            }
        }

        // ── Timeout ──
        GovernanceEntry::TimeoutEntry {
            target,
            duration_seconds,
            started_at,
            lamport,
            ..
        } => {
            let existing_lamport = state.timeouts.get(target).map_or(0, |t| t.lamport);
            if *lamport > existing_lamport {
                state.timeouts.insert(
                    target.clone(),
                    TimeoutState {
                        duration_seconds: *duration_seconds,
                        started_at: *started_at,
                        lamport: *lamport,
                    },
                );
            }
        }

        GovernanceEntry::RemoveTimeoutEntry { target, lamport } => {
            let existing_lamport = state.timeouts.get(target).map_or(0, |t| t.lamport);
            if *lamport > existing_lamport {
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
            lamport,
        } => {
            let existing_lamport = state.automod_rules.get(rule_id).map_or(0, |r| r.lamport);
            if *lamport > existing_lamport {
                state.automod_rules.insert(
                    *rule_id,
                    AutoModRuleState {
                        name: name.clone(),
                        enabled: *enabled,
                        trigger_json: trigger_json.clone(),
                        action: action.clone(),
                        lamport: *lamport,
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
/// `JoinRequested` is a Grow-Only Set insert; the two decision variants
/// are an LWW-Flag per target that removes the pending entry. A decision
/// is idempotent and order-independent — replaying it, or seeing it
/// before the request it answers, converges to the same state, which is
/// what the merge property tests require.
pub(super) fn apply_admission(entry: &GovernanceEntry, state: &mut GovernanceState) {
    match entry {
        GovernanceEntry::JoinRequested {
            requester,
            display_name,
            lamport,
        } => {
            // A decision already recorded at or after this request wins:
            // otherwise a replayed request would resurrect a pending row
            // for someone already approved or rejected.
            if state.admitted.contains_key(requester) {
                return;
            }
            let slot = state
                .pending_members
                .entry(requester.clone())
                .or_insert_with(|| PendingMemberState {
                    display_name: display_name.clone(),
                    lamport: *lamport,
                });
            // Later request from the same pseudonym refreshes the name.
            if *lamport >= slot.lamport {
                slot.display_name.clone_from(display_name);
                slot.lamport = *lamport;
            }
        }
        GovernanceEntry::MemberApproved { target, lamport } => {
            record_decision(state, target, true, *lamport);
        }
        GovernanceEntry::MemberRejected {
            target, lamport, ..
        } => {
            record_decision(state, target, false, *lamport);
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
    lamport: u64,
) {
    let supersedes = state
        .admitted
        .get(target)
        .is_none_or(|existing| lamport >= existing.lamport);
    if supersedes {
        state
            .admitted
            .insert(target.clone(), AdmissionDecision { approved, lamport });
        state.pending_members.remove(target);
    }
}
