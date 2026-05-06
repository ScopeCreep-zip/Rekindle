//! Plate Gate (architecture §15) — multi-segment community membership.
//!
//! When all 255 slots of the highest existing SMPL registry segment are
//! occupied (`seq > 0` for every subkey), an admin (`MANAGE_COMMUNITY`)
//! calls [`expand_community_segment`] to:
//!
//!   1. Create a new pair of SMPL records (registry-(N+1), governance-(N+1))
//!      following the same universal schema as segment 0 (architecture
//!      §15.2).
//!   2. Write a `GovernanceEntry::SegmentAdded` entry announcing the new
//!      segment's keys + slot range; merged across all peers via the
//!      existing CRDT pipeline.
//!
//! CRDT correctness is provided by `rekindle_governance::merge` — segments
//! form an ORMap-of-CRDTs (Shapiro 2011 *CRDTs*; Almeida et al. 2016
//! *Delta State Replicated Data Types*, arXiv:1603.01529, §3) where each
//! segment is its own join-semilattice and the community state is the
//! product CRDT under coordinate-wise join. This module is just the
//! orchestration that wires the new SMPL records into governance.
//!
//! Lazy channel-record creation per-segment (architecture §15.4) is **NOT
//! in scope here** — that's C1-2. v1 of Plate Gate covers membership
//! discovery + admin expansion only.

use rekindle_protocol::dht::community::permissions_v2::Permissions;
use rekindle_records::schema;
use rekindle_secrets::derive;
use rekindle_secrets::keys::SlotSeed;
use rekindle_types::governance::GovernanceEntry;
use veilid_core::CRYPTO_KIND_VLD0;

use crate::state::SharedState;
use crate::state_helpers;

/// Soft cap on segment count. The architecture is silent on a hard maximum
/// (line 2742 references a 1000-member community as 4 segments). Beyond 8
/// (= 2040 members), read amplification on the multi-segment registry scan
/// makes presence-poll latency unworkable without lazy-fetch optimisations
/// (deferred to v2). Increase by changing this constant; no schema change
/// required.
pub const MAX_SEGMENTS: u32 = 8;

/// Local subkey count per segment record (matches the genesis schema in
/// `services/community/create.rs` and architecture §4.6:449).
const SLOTS_PER_SEGMENT: u32 = 255;

/// One row of the segments table — combines the implicit segment 0 (from
/// `CommunityState.{governance_key, member_registry_key}`) with each
/// `SegmentAdded` entry merged from governance.
///
/// Slot ranges are intentionally NOT carried here: they're derivable from
/// `segment_index` since every segment hosts exactly `SLOTS_PER_SEGMENT`
/// (architecture §4.6 universal SMPL schema). The on-the-wire `SegmentAdded`
/// entry keeps `slot_range_start` / `slot_range_end` for forward
/// compatibility with future schemas that might use a different slot count;
/// callers that need the global slot index look at the merged
/// `GovernanceState.segments[i].slot_range_start` directly.
#[derive(Debug, Clone)]
pub struct SegmentDescriptor {
    pub segment_index: u32,
    pub registry_key: String,
    pub governance_key: String,
}

/// Snapshot all segments for a community: the implicit segment 0 + every
/// `SegmentAdded` discovered in the merged governance state. Sorted by
/// `segment_index`. Returns empty when the community is unknown.
pub fn segment_descriptors(state: &SharedState, community_id: &str) -> Vec<SegmentDescriptor> {
    let communities = state.communities.read();
    let Some(community) = communities.get(community_id) else {
        return Vec::new();
    };

    let mut out: Vec<SegmentDescriptor> = Vec::new();
    if let (Some(gov_key), Some(reg_key)) = (
        community.governance_key.clone(),
        community.member_registry_key.clone(),
    ) {
        out.push(SegmentDescriptor {
            segment_index: 0,
            registry_key: reg_key,
            governance_key: gov_key,
        });
    }
    if let Some(gov_state) = community.governance_state.as_ref() {
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
pub async fn highest_segment_full(
    state: &SharedState,
    community_id: &str,
) -> Result<bool, String> {
    let descriptors = segment_descriptors(state, community_id);
    let Some(highest) = descriptors.last() else {
        return Ok(false);
    };
    let rc = state_helpers::safe_routing_context(state).ok_or("not attached")?;
    let record_key = highest
        .registry_key
        .parse::<veilid_core::RecordKey>()
        .map_err(|e| format!("invalid registry key: {e}"))?;
    let report = rc
        .inspect_dht_record(record_key, None, veilid_core::DHTReportScope::Local)
        .await
        .map_err(|e| format!("inspect highest registry: {e}"))?;
    let seqs = report.network_seqs();
    let occupied = seqs
        .iter()
        .take(SLOTS_PER_SEGMENT as usize)
        .filter(|seq| **seq != veilid_core::ValueSeqNum::default())
        .count();
    Ok(occupied >= SLOTS_PER_SEGMENT as usize)
}

/// Phase 1+2 of architecture §15: an admin creates the next SMPL pair
/// (registry + governance) and writes a `SegmentAdded` governance entry.
/// Returns the new `segment_index`.
///
/// Permission: `MANAGE_COMMUNITY` (validated both here and in
/// `rekindle_governance::validate::validate_write` — reader-validates
/// double-checks at every peer per architecture §15.2).
pub async fn expand_community_segment(
    state: &SharedState,
    community_id: &str,
) -> Result<u32, String> {
    crate::commands::community::require_permission(state, community_id, Permissions::MANAGE_COMMUNITY)?;

    let descriptors = segment_descriptors(state, community_id);
    let next_segment_index = descriptors
        .last()
        .map_or(1, |d| d.segment_index + 1);
    if next_segment_index >= MAX_SEGMENTS {
        return Err(format!(
            "segment cap reached ({MAX_SEGMENTS}); raise MAX_SEGMENTS once lazy-fetch lands"
        ));
    }

    if !highest_segment_full(state, community_id).await? {
        return Err(
            "current segment still has open slots — expansion is only allowed when full"
                .to_string(),
        );
    }

    let slot_range_start = next_segment_index * SLOTS_PER_SEGMENT;
    let slot_range_end = slot_range_start + SLOTS_PER_SEGMENT;

    // Slot pubkeys for the new segment use *global* indices per architecture
    // §8.3 + §15.2 (slot 255..509 for segment 1, etc.). Same slot_seed as
    // segment 0 because the seed is community-wide.
    let slot_seed = slot_seed_from_state(state, community_id)?;
    let mut member_pubkeys = Vec::with_capacity(SLOTS_PER_SEGMENT as usize);
    for global_slot in slot_range_start..slot_range_end {
        let sk = derive::derive_slot_keypair(&slot_seed.0, global_slot)
            .map_err(|e| format!("derive_slot_keypair {global_slot}: {e}"))?;
        member_pubkeys.push(sk.verifying_key().to_bytes());
    }

    let rc = state_helpers::safe_routing_context(state).ok_or("not attached")?;
    let new_gov_key = create_segment_smpl_record(&rc, &member_pubkeys).await?;
    let new_reg_key = create_segment_smpl_record(&rc, &member_pubkeys).await?;

    let lamport = state_helpers::increment_lamport(state, community_id);
    crate::services::community::write_entry(
        state,
        community_id,
        GovernanceEntry::SegmentAdded {
            segment_index: next_segment_index,
            registry_key: new_reg_key,
            governance_key: new_gov_key,
            slot_range_start,
            slot_range_end,
            lamport,
        },
    )
    .await?;

    Ok(next_segment_index)
}

/// Open every segment's SMPL records (registry + governance + per-channel
/// segment records) that the merged `GovernanceState` now contains but our
/// local `open_community_records` hasn't recorded yet. Called from
/// `state_helpers::set_governance_state` after every successful merge so
/// that `get_dht_value`/`watch_dht_values` + the inspect loop pick up
/// new segments + lazy channel-segment records immediately. Idempotent —
/// DHT `open_record` is safe to call repeatedly.
pub async fn open_new_segments(state: &SharedState, community_id: &str) {
    let descriptors = segment_descriptors(state, community_id);
    let already_open: std::collections::HashSet<String> = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .map(|c| {
                c.open_community_records
                    .channel_keys
                    .iter()
                    .cloned()
                    .chain(c.open_community_records.registry_key.iter().cloned())
                    .chain(c.open_community_records.governance_key.iter().cloned())
                    .collect()
            })
            .unwrap_or_default()
    };
    let Some(rc) = state_helpers::safe_routing_context(state).clone() else {
        return;
    };

    // Expansion segments: registry + governance records.
    for descriptor in &descriptors {
        if descriptor.segment_index == 0 {
            continue; // primary segment was opened during join/genesis
        }
        for key in [&descriptor.registry_key, &descriptor.governance_key] {
            if already_open.contains(key) {
                continue;
            }
            let Ok(record_key) = key.parse::<veilid_core::RecordKey>() else {
                continue;
            };
            if let Err(e) = rc.open_dht_record(record_key, None).await {
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

    // Plate Gate (architecture §15.4): channel-segment records announced via
    // `ChannelSegmentLinked`. Each opens just like a normal channel record;
    // also tracked in `open_community_records.channel_keys` so the inspect
    // loop picks them up automatically.
    let channel_segment_keys: Vec<String> = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .and_then(|c| c.governance_state.as_ref())
            .map(|gov| {
                gov.channel_segment_records
                    .values()
                    .map(|rec| rec.record_key.clone())
                    .collect()
            })
            .unwrap_or_default()
    };
    for key in channel_segment_keys {
        if already_open.contains(&key) {
            continue;
        }
        let Ok(record_key) = key.parse::<veilid_core::RecordKey>() else {
            continue;
        };
        if let Err(e) = rc.open_dht_record(record_key, None).await {
            tracing::debug!(
                community = %community_id,
                record_key = %key,
                error = %e,
                "open_new_segments: failed to open lazy channel-segment record"
            );
            continue;
        }
        // Track for the inspect/watch loops.
        let mut communities = state.communities.write();
        if let Some(cs) = communities.get_mut(community_id) {
            if !cs.open_community_records.channel_keys.contains(&key) {
                cs.open_community_records.channel_keys.push(key);
            }
        }
    }
}

/// Ensure a segment-N member has a channel-segment SMPL record to write
/// to (architecture §15.4 lazy channel records). Returns the record key.
///
/// For segment-0 callers this is a fast path — the genesis channel record
/// already exists and is returned from `community.channel_log_keys`. For
/// segment-N (N>0) callers this checks `governance_state.channel_segment_records`;
/// if no entry exists yet, creates a fresh SMPL record (same universal
/// schema as segment 0), writes a `ChannelSegmentLinked` governance entry
/// announcing it, and returns the new key.
///
/// Race semantics: if two segment-N members race to send the first message
/// in `(channel_id, segment_index)`, both will create records. The CRDT
/// merge picks one canonical record via LWW (`linked_lamport`); subsequent
/// writes from both members go to that canonical record once the loser's
/// merge applies. The race-loser's first message lives in their orphan
/// record — readers (when generalized in the read paths) scan all
/// `ChannelSegmentLinked` records to surface them. v1 acceptable since
/// the race window is microseconds-wide and only on the very first
/// segment-N message in a channel.
pub async fn ensure_channel_segment_record(
    state: &SharedState,
    community_id: &str,
    channel_id: &str,
) -> Result<String, String> {
    let (segment_index, existing_record, channel_id_bytes, slot_seed_present) = {
        let communities = state.communities.read();
        let community = communities
            .get(community_id)
            .ok_or_else(|| "community not found".to_string())?;
        let segment_index = community.my_segment_index.unwrap_or(0);
        if segment_index == 0 {
            // Segment 0 fast path — the genesis record always exists.
            return community
                .channel_log_keys
                .get(channel_id)
                .cloned()
                .ok_or_else(|| "channel record key missing".to_string());
        }
        let channel_id_bytes: [u8; 16] = hex::decode(channel_id)
            .ok()
            .and_then(|b| b.try_into().ok())
            .unwrap_or([0u8; 16]);
        let channel_id_typed = rekindle_types::id::ChannelId(channel_id_bytes);
        let existing = community.governance_state.as_ref().and_then(|gov| {
            gov.channel_segment_records
                .get(&(channel_id_typed, segment_index))
                .map(|rec| rec.record_key.clone())
        });
        (
            segment_index,
            existing,
            channel_id_bytes,
            community.slot_seed.is_some(),
        )
    };

    if let Some(record_key) = existing_record {
        return Ok(record_key);
    }
    if !slot_seed_present {
        return Err(
            "slot_seed missing — segment-N channel-record creation needs the seed for slot keypair derivation"
                .to_string(),
        );
    }

    // Lazy creation: derive 255 slot pubkeys for this segment using GLOBAL
    // slot indices (architecture §8.3 + §15.2), build the universal SMPL
    // schema, create the record.
    let slot_seed = slot_seed_from_state(state, community_id)?;
    let slot_range_start = segment_index * SLOTS_PER_SEGMENT;
    let mut member_pubkeys = Vec::with_capacity(SLOTS_PER_SEGMENT as usize);
    for global_slot in slot_range_start..slot_range_start + SLOTS_PER_SEGMENT {
        let sk = derive::derive_slot_keypair(&slot_seed.0, global_slot)
            .map_err(|e| format!("derive_slot_keypair {global_slot}: {e}"))?;
        member_pubkeys.push(sk.verifying_key().to_bytes());
    }

    let rc = state_helpers::safe_routing_context(state).ok_or("not attached")?;
    let new_record_key = create_segment_smpl_record(&rc, &member_pubkeys).await?;

    // Announce via governance — first-writer wins LWW, but if we lose the
    // race we still proceed: messages get written to our orphan record,
    // reads from peers happen when their merge applies. Re-aim our
    // subsequent writes is handled by the next call (which sees the merged
    // canonical record).
    let lamport = state_helpers::increment_lamport(state, community_id);
    crate::services::community::write_entry(
        state,
        community_id,
        rekindle_types::governance::GovernanceEntry::ChannelSegmentLinked {
            channel_id: rekindle_types::id::ChannelId(channel_id_bytes),
            segment_index,
            record_key: new_record_key.clone(),
            lamport,
        },
    )
    .await?;

    Ok(new_record_key)
}

/// Collect every channel SMPL record key for a (community, channel):
/// the segment-0 genesis record + every `ChannelSegmentLinked` that has
/// merged into governance state. Used by the multi-segment read paths
/// (presence sync, message-notification fetch, history catchup) so
/// segment-N peers' messages are discoverable.
pub fn channel_record_keys_per_segment(
    state: &SharedState,
    community_id: &str,
    channel_id: &str,
) -> Vec<(u32, String)> {
    let communities = state.communities.read();
    let Some(community) = communities.get(community_id) else {
        return Vec::new();
    };
    let mut out: Vec<(u32, String)> = Vec::new();
    if let Some(record_key) = community.channel_log_keys.get(channel_id).cloned() {
        out.push((0, record_key));
    }
    let channel_id_bytes: [u8; 16] = hex::decode(channel_id)
        .ok()
        .and_then(|b| b.try_into().ok())
        .unwrap_or([0u8; 16]);
    let channel_id_typed = rekindle_types::id::ChannelId(channel_id_bytes);
    if let Some(gov) = community.governance_state.as_ref() {
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

/// Read the `slot_seed` from `CommunityState`. The seed is a 32-byte
/// shared secret distributed to all members at join time (alongside the
/// MEK in `JoinAccepted`).
fn slot_seed_from_state(
    state: &SharedState,
    community_id: &str,
) -> Result<SlotSeed, String> {
    let communities = state.communities.read();
    let community = communities
        .get(community_id)
        .ok_or_else(|| "community not found".to_string())?;
    let seed_hex = community
        .slot_seed
        .as_ref()
        .ok_or_else(|| "slot_seed missing — expansion requires creator/admin to hold the seed".to_string())?;
    let seed_bytes: [u8; 32] = hex::decode(seed_hex)
        .map_err(|e| format!("invalid slot_seed hex: {e}"))?
        .try_into()
        .map_err(|_| "slot_seed must be 32 bytes".to_string())?;
    Ok(SlotSeed(seed_bytes))
}

/// Create a new SMPL DHT record using the universal community schema.
/// Mirrors steps 3-4 of `services/community/create.rs::create_community`
/// (genesis path) but for an expansion segment. The owner secret is
/// retained only for `open_record` callbacks; never used for writing
/// (`o_cnt: 0` Schwarzschild principle, architecture §3).
async fn create_segment_smpl_record(
    rc: &veilid_core::RoutingContext,
    member_pubkeys: &[[u8; 32]],
) -> Result<String, String> {
    let smpl_schema = schema::community_smpl_schema(member_pubkeys)
        .map_err(|e| format!("segment schema creation failed: {e}"))?;
    let desc = rc
        .create_dht_record(CRYPTO_KIND_VLD0, smpl_schema, None)
        .await
        .map_err(|e| format!("segment record creation failed: {e}"))?;
    Ok(desc.key().to_string())
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
}
