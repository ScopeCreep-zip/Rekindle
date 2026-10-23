//! `merge` community CRDT apply rules.

use crate::op::OpStamp;

use super::{
    CommunityPolicyState, GovernanceEntry, GovernanceState, InviteState, MetadataState,
    PseudonymKey, SegmentState,
};

pub(super) fn apply_community(
    author: &PseudonymKey,
    entry: &GovernanceEntry,
    stamp: OpStamp,
    state: &mut GovernanceState,
) {
    match entry {
        GovernanceEntry::CommunityMeta {
            name,
            description,
            icon_hash,
            banner_hash,
        } => {
            let existing_stamp = state.metadata.as_ref().map_or(OpStamp::MIN, |m| m.stamp);
            if stamp > existing_stamp {
                state.metadata = Some(MetadataState {
                    name: name.clone().unwrap_or_default(),
                    description: description.clone(),
                    icon_hash: icon_hash.clone(),
                    banner_hash: banner_hash.clone(),
                    stamp,
                });
            }
        }

        // ── Notification default (architecture §17.1 tier 1): LWW ──
        GovernanceEntry::CommunityNotificationDefault { level } => {
            let existing_stamp = state
                .notification_default
                .as_ref()
                .map_or(OpStamp::MIN, |d| d.stamp);
            if stamp > existing_stamp {
                state.notification_default = Some(crate::state::NotificationDefaultState {
                    level: level.clone(),
                    stamp,
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
        } => {
            let existing_stamp = state
                .community_policy
                .as_ref()
                .map_or(OpStamp::MIN, |p| p.stamp);
            if stamp > existing_stamp {
                state.community_policy = Some(CommunityPolicyState {
                    policy_text: policy_text.clone(),
                    max_joins_per_interval: *max_joins_per_interval,
                    join_interval_seconds: *join_interval_seconds,
                    stamp,
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
        } => {
            state.invites.insert(
                *invite_id,
                InviteState {
                    code_hash: code_hash.clone(),
                    max_uses: *max_uses,
                    expires_at: *expires_at,
                    secrets_record_key: secrets_record_key.clone(),
                    created: stamp,
                    creator_pseudonym: author.clone(),
                },
            );
        }

        GovernanceEntry::InviteRevoked { invite_id, .. } => {
            // Only revoke if the revocation lamport > creation lamport
            if let Some(invite) = state.invites.get(invite_id) {
                if stamp > invite.created {
                    state.invites.remove(invite_id);
                }
            }
        }
        _ => unreachable!("apply_community: unexpected variant"),
    }
}
