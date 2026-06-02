//! `merge` moderation CRDT apply rules.

use super::*;

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
            let existing_lamport = state.timeouts.get(target).map(|t| t.lamport).unwrap_or(0);
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
            let existing_lamport = state.timeouts.get(target).map(|t| t.lamport).unwrap_or(0);
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
            let existing_lamport = state
                .automod_rules
                .get(rule_id)
                .map(|r| r.lamport)
                .unwrap_or(0);
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
