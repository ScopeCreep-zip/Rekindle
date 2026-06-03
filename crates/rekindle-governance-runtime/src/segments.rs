//! Plate Gate — architecture §15 multi-segment community membership.
//!
//! Ported from `src-tauri/src/services/community/segments.rs`.
//!
//! When all 255 slots of the highest existing SMPL registry segment
//! are occupied (`seq > 0` for every subkey), an admin
//! (`MANAGE_COMMUNITY`) calls [`expand_community_segment`] to:
//!
//!   1. Create a new pair of SMPL records (registry-(N+1), governance-(N+1))
//!      following the same universal schema as segment 0.
//!   2. Write a `GovernanceEntry::SegmentAdded` entry announcing the new
//!      segment's keys + slot range, merged across all peers via the
//!      existing CRDT pipeline.
//!
//! CRDT correctness is provided by `rekindle_governance::merge` — segments
//! form an ORMap-of-CRDTs (Shapiro 2011 *CRDTs*; Almeida et al. 2016
//! *Delta State Replicated Data Types*) where each segment is its own
//! join-semilattice and the community state is the product CRDT under
//! coordinate-wise join. This module is just the orchestration that wires
//! the new SMPL records into governance.

use rekindle_protocol::dht::community::permissions_v2::Permissions;
use rekindle_secrets::derive;
use rekindle_secrets::keys::SlotSeed;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::ChannelId;

use crate::apply;
use crate::deps::{CommunityMembership, GovernanceRuntimeDeps};
use crate::error::GovernanceRuntimeError;
use crate::event::GovernanceRuntimeEvent;

/// Soft cap on segment count. Beyond 8 segments (= 2040 members) read
/// amplification on the multi-segment registry scan makes presence-poll
/// latency unworkable without lazy-fetch optimisations (deferred to v2).
pub const MAX_SEGMENTS: u32 = 8;

/// Local subkey count per segment record (matches genesis SMPL schema
/// in `origin.rs` and architecture §4.6:449).
const SLOTS_PER_SEGMENT: u32 = 255;

/// One row of the segments table — combines the implicit segment 0
/// (from `CommunityMembership.governance_key + member_registry_key`)
/// with each `SegmentAdded` entry merged from governance.
#[derive(Debug, Clone)]
pub struct SegmentDescriptor {
    pub segment_index: u32,
    pub registry_key: String,
    pub governance_key: String,
}

/// Snapshot all segments for a community: the implicit segment 0 + every
/// `SegmentAdded` discovered in the merged governance state. Sorted by
/// `segment_index`. Returns empty when the community is unknown.
pub fn segment_descriptors<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
) -> Vec<SegmentDescriptor> {
    let Some(membership) = deps.community_membership(community_id) else {
        return Vec::new();
    };

    let mut out: Vec<SegmentDescriptor> = Vec::new();
    if let (Some(gov_key), Some(reg_key)) = (
        membership.governance_key.clone(),
        membership.member_registry_key.clone(),
    ) {
        out.push(SegmentDescriptor {
            segment_index: 0,
            registry_key: reg_key,
            governance_key: gov_key,
        });
    }
    if let Some(gov_state) = deps.governance_state(community_id) {
        for seg in &gov_state.segments {
            if seg.segment_index == 0 {
                continue; // segment 0 is implicit — never re-announced
            }
            out.push(SegmentDescriptor {
                segment_index: seg.segment_index,
                registry_key: seg.registry_key.clone(),
                governance_key: seg.governance_key.clone(),
            });
        }
    }
    out.sort_by_key(|d| d.segment_index);
    out
}

/// Check whether the highest existing segment has every local subkey
/// occupied (`seq > 0`). Architecture §15.1 trigger condition.
pub async fn highest_segment_full<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
) -> Result<bool, GovernanceRuntimeError> {
    let descriptors = segment_descriptors(deps, community_id);
    let Some(highest) = descriptors.last() else {
        return Ok(false);
    };
    let seqs = deps
        .inspect_dht_record_local_seqs(&highest.registry_key)
        .await?;
    let occupied = seqs
        .iter()
        .take(SLOTS_PER_SEGMENT as usize)
        .filter(|seq| **seq != 0)
        .count();
    Ok(occupied >= SLOTS_PER_SEGMENT as usize)
}

/// Phase 1+2 of architecture §15: an admin creates the next SMPL pair
/// (registry + governance) and writes a `SegmentAdded` governance entry.
/// Returns the new `segment_index`.
///
/// Permission: `MANAGE_COMMUNITY` (validated both here and inside
/// `rekindle_governance::validate::validate_write` — reader-validates
/// double-checks at every peer per architecture §15.2).
pub async fn expand_community_segment<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
) -> Result<u32, GovernanceRuntimeError> {
    deps.require_permission(community_id, Permissions::MANAGE_COMMUNITY.bits())?;

    let descriptors = segment_descriptors(deps, community_id);
    let next_segment_index = descriptors.last().map_or(1, |d| d.segment_index + 1);
    if next_segment_index >= MAX_SEGMENTS {
        return Err(GovernanceRuntimeError::SegmentCapReached(MAX_SEGMENTS));
    }

    if !highest_segment_full(deps, community_id).await? {
        return Err(GovernanceRuntimeError::SegmentNotFull);
    }

    let slot_range_start = next_segment_index * SLOTS_PER_SEGMENT;
    let slot_range_end = slot_range_start + SLOTS_PER_SEGMENT;

    // Slot pubkeys for the new segment use *global* indices per architecture
    // §8.3 + §15.2 (slot 255..509 for segment 1, etc.). Same slot_seed as
    // segment 0 because the seed is community-wide.
    let membership = deps
        .community_membership(community_id)
        .ok_or_else(|| GovernanceRuntimeError::CommunityNotFound(community_id.to_string()))?;
    let slot_seed = slot_seed_from_membership(&membership, community_id)?;
    let mut member_pubkeys = Vec::with_capacity(SLOTS_PER_SEGMENT as usize);
    for global_slot in slot_range_start..slot_range_end {
        let sk = derive::derive_slot_keypair(&slot_seed.0, global_slot).map_err(|e| {
            GovernanceRuntimeError::Crypto(format!("derive_slot_keypair {global_slot}: {e}"))
        })?;
        member_pubkeys.push(sk.verifying_key().to_bytes());
    }

    let new_gov_record = deps.create_smpl_record(&member_pubkeys).await?;
    let new_reg_record = deps.create_smpl_record(&member_pubkeys).await?;

    let lamport = deps.increment_lamport(community_id);
    apply::write_entry(
        deps,
        community_id,
        GovernanceEntry::SegmentAdded {
            segment_index: next_segment_index,
            registry_key: new_reg_record.record_key,
            governance_key: new_gov_record.record_key,
            slot_range_start,
            slot_range_end,
            lamport,
        },
    )
    .await?;

    deps.emit_event(GovernanceRuntimeEvent::SegmentAdded {
        community_id: community_id.to_string(),
        segment_index: next_segment_index,
    });

    Ok(next_segment_index)
}

/// Open every segment's SMPL records (registry + governance + per-channel
/// segment records) that the merged `GovernanceState` now contains but
/// our local `open_community_records` hasn't recorded yet. Called from
/// the adapter after every successful merge so `get_dht_value` /
/// `watch_dht_values` + the inspect loop pick up new segments + lazy
/// channel-segment records immediately. Idempotent — DHT `open_record`
/// is safe to call repeatedly.
pub async fn open_new_segments<D: GovernanceRuntimeDeps>(deps: &D, community_id: &str) {
    let descriptors = segment_descriptors(deps, community_id);
    let already_open: std::collections::HashSet<String> =
        deps.open_record_keys(community_id).into_iter().collect();

    // Expansion segments: registry + governance records.
    for descriptor in &descriptors {
        if descriptor.segment_index == 0 {
            continue; // primary segment was opened during join/genesis
        }
        for key in [&descriptor.registry_key, &descriptor.governance_key] {
            if already_open.contains(key) {
                continue;
            }
            if let Err(e) = deps.open_dht_record(key, None).await {
                tracing::debug!(
                    community = %community_id,
                    segment = descriptor.segment_index,
                    record_key = %key,
                    error = %e,
                    "open_new_segments: failed to open expansion record"
                );
            }
        }
    }

    // Plate Gate (architecture §15.4): channel-segment records announced
    // via `ChannelSegmentLinked`. Each opens just like a normal channel
    // record; also tracked in `open_community_records.channel_keys` so the
    // inspect loop picks them up automatically.
    let channel_segment_keys: Vec<String> = deps
        .governance_state(community_id)
        .map(|gov| {
            gov.channel_segment_records
                .values()
                .map(|rec| rec.record_key.clone())
                .collect()
        })
        .unwrap_or_default();
    for key in channel_segment_keys {
        if already_open.contains(&key) {
            continue;
        }
        if let Err(e) = deps.open_dht_record(&key, None).await {
            tracing::debug!(
                community = %community_id,
                record_key = %key,
                error = %e,
                "open_new_segments: failed to open lazy channel-segment record"
            );
            continue;
        }
        deps.mark_open_channel_record(community_id, key);
    }
}

/// Ensure a segment-N member has a channel-segment SMPL record to write
/// to (architecture §15.4 lazy channel records). Returns the record key.
///
/// For segment-0 callers this is a fast path — the genesis channel record
/// already exists and is returned from `CommunityMembership.channel_log_keys`.
/// For segment-N (N>0) callers this checks
/// `governance_state.channel_segment_records`; if no entry exists yet,
/// creates a fresh SMPL record (same universal schema as segment 0),
/// writes a `ChannelSegmentLinked` governance entry announcing it, and
/// returns the new key.
pub async fn ensure_channel_segment_record<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
) -> Result<String, GovernanceRuntimeError> {
    let membership = deps
        .community_membership(community_id)
        .ok_or_else(|| GovernanceRuntimeError::CommunityNotFound(community_id.to_string()))?;
    let segment_index = membership.my_segment_index.unwrap_or(0);
    if segment_index == 0 {
        // Segment 0 fast path — the genesis record always exists.
        return membership
            .channel_log_keys
            .get(channel_id)
            .cloned()
            .ok_or_else(|| {
                GovernanceRuntimeError::Adapter("channel record key missing".to_string())
            });
    }

    let channel_id_bytes: [u8; 16] = hex::decode(channel_id)
        .ok()
        .and_then(|b| b.try_into().ok())
        .unwrap_or([0u8; 16]);
    let channel_id_typed = ChannelId(channel_id_bytes);

    let existing = deps.governance_state(community_id).and_then(|gov| {
        gov.channel_segment_records
            .get(&(channel_id_typed, segment_index))
            .map(|rec| rec.record_key.clone())
    });
    if let Some(record_key) = existing {
        return Ok(record_key);
    }
    if membership.slot_seed_hex.is_none() {
        return Err(GovernanceRuntimeError::SlotSeedMissing(
            community_id.to_string(),
        ));
    }

    // Lazy creation: derive 255 slot pubkeys for this segment using GLOBAL
    // slot indices (architecture §8.3 + §15.2), build the universal SMPL
    // schema, create the record.
    let slot_seed = slot_seed_from_membership(&membership, community_id)?;
    let slot_range_start = segment_index * SLOTS_PER_SEGMENT;
    let mut member_pubkeys = Vec::with_capacity(SLOTS_PER_SEGMENT as usize);
    for global_slot in slot_range_start..slot_range_start + SLOTS_PER_SEGMENT {
        let sk = derive::derive_slot_keypair(&slot_seed.0, global_slot).map_err(|e| {
            GovernanceRuntimeError::Crypto(format!("derive_slot_keypair {global_slot}: {e}"))
        })?;
        member_pubkeys.push(sk.verifying_key().to_bytes());
    }

    let new_record = deps.create_smpl_record(&member_pubkeys).await?;
    let new_record_key = new_record.record_key.clone();

    // Announce via governance — first-writer-wins LWW. If we lose the
    // race, messages go to our orphan record; readers pick up the
    // canonical record once the merge applies.
    let lamport = deps.increment_lamport(community_id);
    apply::write_entry(
        deps,
        community_id,
        GovernanceEntry::ChannelSegmentLinked {
            channel_id: channel_id_typed,
            segment_index,
            record_key: new_record_key.clone(),
            lamport,
        },
    )
    .await?;

    Ok(new_record_key)
}

/// Collect every channel SMPL record key for a `(community, channel)`:
/// the segment-0 genesis record + every `ChannelSegmentLinked` that has
/// merged into governance state. Used by multi-segment read paths
/// (presence sync, message-notification fetch, history catchup) so
/// segment-N peers' messages are discoverable.
pub fn channel_record_keys_per_segment<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
) -> Vec<(u32, String)> {
    let Some(membership) = deps.community_membership(community_id) else {
        return Vec::new();
    };
    let mut out: Vec<(u32, String)> = Vec::new();
    if let Some(record_key) = membership.channel_log_keys.get(channel_id).cloned() {
        out.push((0, record_key));
    }
    let channel_id_bytes: [u8; 16] = hex::decode(channel_id)
        .ok()
        .and_then(|b| b.try_into().ok())
        .unwrap_or([0u8; 16]);
    let channel_id_typed = ChannelId(channel_id_bytes);
    if let Some(gov) = deps.governance_state(community_id) {
        for ((cid, seg_idx), record) in &gov.channel_segment_records {
            if *cid != channel_id_typed {
                continue;
            }
            if *seg_idx == 0 {
                continue; // segment 0 already handled
            }
            out.push((*seg_idx, record.record_key.clone()));
        }
    }
    out.sort_by_key(|(idx, _)| *idx);
    out
}

/// Read the `slot_seed` from `CommunityMembership`. The seed is a 32-byte
/// shared secret distributed to all members at join time (alongside the
/// MEK in `JoinAccepted`).
fn slot_seed_from_membership(
    membership: &CommunityMembership,
    community_id: &str,
) -> Result<SlotSeed, GovernanceRuntimeError> {
    let seed_hex = membership
        .slot_seed_hex
        .as_ref()
        .ok_or_else(|| GovernanceRuntimeError::SlotSeedMissing(community_id.to_string()))?;
    let seed_bytes: [u8; 32] = hex::decode(seed_hex)
        .map_err(|e| GovernanceRuntimeError::Crypto(format!("invalid slot_seed hex: {e}")))?
        .try_into()
        .map_err(|_| GovernanceRuntimeError::Crypto("slot_seed must be 32 bytes".to_string()))?;
    Ok(SlotSeed(seed_bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_range_arithmetic() {
        // Segment 1 hosts slots 255..510, segment 2: 510..765 — matches
        // architecture §15.2 example.
        let segment_1_start = SLOTS_PER_SEGMENT;
        let segment_1_end = segment_1_start + SLOTS_PER_SEGMENT;
        assert_eq!(segment_1_start, 255);
        assert_eq!(segment_1_end, 510);

        let segment_2_start = 2 * SLOTS_PER_SEGMENT;
        let segment_2_end = segment_2_start + SLOTS_PER_SEGMENT;
        assert_eq!(segment_2_start, 510);
        assert_eq!(segment_2_end, 765);
    }

    #[test]
    fn max_segments_default_is_eight() {
        // Capacity check: MAX_SEGMENTS * SLOTS_PER_SEGMENT = 2040 members.
        assert_eq!(MAX_SEGMENTS, 8);
        assert_eq!(MAX_SEGMENTS * SLOTS_PER_SEGMENT, 2040);
    }

    #[test]
    fn slot_seed_from_membership_decodes_hex() {
        let membership = CommunityMembership {
            governance_key: None,
            member_registry_key: None,
            my_pseudonym_hex: None,
            my_subkey_index: None,
            my_segment_index: None,
            slot_keypair: None,
            slot_seed_hex: Some(hex::encode([5u8; 32])),
            dht_owner_keypair: None,
            lamport_counter: 0,
            channel_log_keys: std::collections::HashMap::new(),
            channel_ids: Vec::new(),
            mek_generation: 0,
        };
        let seed = slot_seed_from_membership(&membership, "test").expect("decode");
        assert_eq!(seed.0, [5u8; 32]);
    }

    #[test]
    fn slot_seed_from_membership_missing_errors() {
        let membership = CommunityMembership {
            governance_key: None,
            member_registry_key: None,
            my_pseudonym_hex: None,
            my_subkey_index: None,
            my_segment_index: None,
            slot_keypair: None,
            slot_seed_hex: None,
            dht_owner_keypair: None,
            lamport_counter: 0,
            channel_log_keys: std::collections::HashMap::new(),
            channel_ids: Vec::new(),
            mek_generation: 0,
        };
        assert!(matches!(
            slot_seed_from_membership(&membership, "test"),
            Err(GovernanceRuntimeError::SlotSeedMissing(_))
        ));
    }
}
