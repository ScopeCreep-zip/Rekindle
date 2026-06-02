//! `merge` community CRDT apply rules.

use super::*;

pub(super) fn apply_community(
    author: &PseudonymKey,
    entry: &GovernanceEntry,
    state: &mut GovernanceState,
) {
    match entry {
        GovernanceEntry::CommunityMeta {
            name,
            description,
            icon_hash,
            banner_hash,
            lamport,
        } => {
            let existing_lamport = state.metadata.as_ref().map(|m| m.lamport).unwrap_or(0);
            if *lamport > existing_lamport {
                state.metadata = Some(MetadataState {
                    name: name.clone().unwrap_or_default(),
                    description: description.clone(),
                    icon_hash: icon_hash.clone(),
                    banner_hash: banner_hash.clone(),
                    lamport: *lamport,
                });
            }
        }

        // ── Notification default (architecture §17.1 tier 1): LWW ──
        GovernanceEntry::CommunityNotificationDefault { level, lamport } => {
            let existing_lamport = state.notification_default.as_ref().map_or(0, |d| d.lamport);
            if *lamport > existing_lamport {
                state.notification_default = Some(crate::state::NotificationDefaultState {
                    level: level.clone(),
                    lamport: *lamport,
                });
            }
        }

        // ── MEK: Max-Register ──
        GovernanceEntry::MEKGenerationBump { generation, .. } => {
            if *generation > state.mek_generation {
                state.mek_generation = *generation;
            }
        }

        // ── Categories: OR-Set ──
        GovernanceEntry::SegmentAdded {
            segment_index,
            registry_key,
            governance_key,
            slot_range_start,
            slot_range_end,
            ..
        } => {
            // Avoid duplicates — segment_index is unique
            if !state
                .segments
                .iter()
                .any(|s| s.segment_index == *segment_index)
            {
                state.segments.push(SegmentState {
                    segment_index: *segment_index,
                    registry_key: registry_key.clone(),
                    governance_key: governance_key.clone(),
                    slot_range_start: *slot_range_start,
                    slot_range_end: *slot_range_end,
                });
                state.segments.sort_by_key(|s| s.segment_index);
            }
        }

        // ── Community-wide policy (rules text + raid thresholds) ──
        GovernanceEntry::CommunityPolicy {
            policy_text,
            max_joins_per_interval,
            join_interval_seconds,
            lamport,
        } => {
            let existing_lamport = state
                .community_policy
                .as_ref()
                .map(|p| p.lamport)
                .unwrap_or(0);
            if *lamport > existing_lamport {
                state.community_policy = Some(CommunityPolicyState {
                    policy_text: policy_text.clone(),
                    max_joins_per_interval: *max_joins_per_interval,
                    join_interval_seconds: *join_interval_seconds,
                    lamport: *lamport,
                });
            }
        }

        // ── AutoMod rules: LWW per rule_id ──
        GovernanceEntry::InviteCreated {
            invite_id,
            code_hash,
            max_uses,
            expires_at,
            secrets_record_key,
            lamport,
        } => {
            state.invites.insert(
                *invite_id,
                InviteState {
                    code_hash: code_hash.clone(),
                    max_uses: *max_uses,
                    expires_at: *expires_at,
                    secrets_record_key: secrets_record_key.clone(),
                    created_lamport: *lamport,
                    creator_pseudonym: author.clone(),
                },
            );
        }

        GovernanceEntry::InviteRevoked {
            invite_id, lamport, ..
        } => {
            // Only revoke if the revocation lamport > creation lamport
            if let Some(invite) = state.invites.get(invite_id) {
                if *lamport > invite.created_lamport {
                    state.invites.remove(invite_id);
                }
            }
        }
        _ => unreachable!("apply_community: unexpected variant"),
    }
}
