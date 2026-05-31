//! State-read helpers for `CommunityPresenceDeps` — `ensure_registry_open`,
//! `presence_credentials`, `governance_bans`, `segment_descriptors`.
//!
//! Pre-port these reads were inline inside `presence_poll_tick`;
//! lifted here so each adapter method body stays one-liner thin
//! (per the no-god-modules rule).

use std::collections::HashSet;
use std::sync::Arc;

use rekindle_presence::{PresenceCredentials, SegmentDescriptor};
use rekindle_protocol::dht::DHTManager;

use crate::services::community::join::try_derive_slot_keypair;
use crate::state::AppState;
use crate::state_helpers;

pub(super) async fn ensure_registry_open(
    state: &Arc<AppState>,
    community_id: &str,
) -> Result<Option<String>, String> {
    let registry_key = {
        let communities = state.communities.read();
        let c = communities.get(community_id).ok_or("community not found")?;
        c.member_registry_key.clone()
    };
    let Some(registry_key) = registry_key else {
        return Ok(None);
    };
    let rc = state_helpers::safe_routing_context(state).ok_or("not attached")?;
    let mgr = DHTManager::new(rc);
    crate::services::community::presence::registry::ensure_registry_open(
        state,
        community_id,
        &mgr,
        &registry_key,
    )
    .await?;
    Ok(Some(registry_key))
}

pub(super) fn presence_credentials(
    state: &Arc<AppState>,
    community_id: &str,
) -> Option<PresenceCredentials> {
    let (my_pseudonym_hex, my_subkey_index, slot_keypair_str, slot_seed_hex, my_segment_index) = {
        let communities = state.communities.read();
        let c = communities.get(community_id)?;
        (
            c.my_pseudonym_key.clone().unwrap_or_default(),
            c.my_subkey_index,
            c.slot_keypair.clone(),
            c.slot_seed.clone(),
            c.my_segment_index.unwrap_or(0),
        )
    };

    // Lazy slot-keypair derivation — pre-port did this inline in
    // `presence_poll_tick` so a returning member whose keypair
    // hadn't been re-derived after restart could still publish.
    let slot_keypair_str = if slot_keypair_str.is_none() {
        if let (Some(ref seed_hex), Some(subkey_idx)) = (&slot_seed_hex, my_subkey_index) {
            try_derive_slot_keypair(state, community_id, seed_hex, subkey_idx)
        } else {
            None
        }
    } else {
        slot_keypair_str
    };

    Some(PresenceCredentials {
        my_pseudonym_hex,
        my_subkey_index,
        slot_keypair_str,
        slot_seed_hex,
        my_segment_index,
    })
}

pub(super) fn governance_bans(state: &Arc<AppState>, community_id: &str) -> HashSet<String> {
    state_helpers::governance_state(state, community_id)
        .map(|gov_state| {
            gov_state
                .bans
                .iter()
                .map(|pseudo| hex::encode(pseudo.0))
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn segment_descriptors(
    state: &Arc<AppState>,
    community_id: &str,
) -> Vec<SegmentDescriptor> {
    crate::services::community::segments::segment_descriptors(state, community_id)
        .into_iter()
        .map(|d| SegmentDescriptor {
            segment_index: d.segment_index,
            registry_key: d.registry_key,
        })
        .collect()
}

// ---- Per-community read shortcuts used by community_deps.rs ----

pub(super) fn my_pseudonym_for_community(state: &Arc<AppState>, community_id: &str) -> String {
    state
        .communities
        .read()
        .get(community_id)
        .and_then(|c| c.my_pseudonym_key.clone())
        .unwrap_or_default()
}

pub(super) fn channel_ids_for_community(state: &Arc<AppState>, community_id: &str) -> Vec<String> {
    state
        .communities
        .read()
        .get(community_id)
        .map(|c| c.channels.iter().map(|ch| ch.id.clone()).collect())
        .unwrap_or_default()
}

pub(super) fn channel_log_keys_for_community(
    state: &Arc<AppState>,
    community_id: &str,
) -> Vec<(String, String)> {
    state
        .communities
        .read()
        .get(community_id)
        .map(|c| {
            c.channel_log_keys
                .iter()
                .map(|(ch_id, rec)| (ch_id.clone(), rec.clone()))
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn member_count_for_community(state: &Arc<AppState>, community_id: &str) -> u32 {
    state
        .communities
        .read()
        .get(community_id)
        .map_or(0, |c| u32::try_from(c.known_members.len()).unwrap_or(255))
}

pub(super) fn mark_pending_sync(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
    attempt: u32,
) {
    let now = rekindle_utils::timestamp_secs();
    if let Some(c) = state.communities.write().get_mut(community_id) {
        c.pending_syncs
            .insert(channel_id.to_string(), (now, attempt));
    }
}

pub(super) fn mark_initial_sync_done(state: &Arc<AppState>, community_id: &str) {
    if let Some(c) = state.communities.write().get_mut(community_id) {
        if let Some(ref mut g) = c.gossip {
            g.needs_initial_sync = false;
        }
    }
}
