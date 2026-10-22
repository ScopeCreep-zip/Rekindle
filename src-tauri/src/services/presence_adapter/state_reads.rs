//! State-read helpers for `CommunityPresenceDeps` — `ensure_registry_open`,
//! `presence_credentials`, `governance_bans`, `segment_descriptors`.
//!
//! Pre-port these reads were inline inside `presence_poll_tick`;
//! lifted here so each adapter method body stays one-liner thin
//! (per the no-god-modules rule).

use std::collections::HashSet;
use std::sync::Arc;

use rekindle_presence::{PresenceCredentials, SegmentDescriptor};
use rekindle_secrets::derive;

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
    crate::services::community::presence::registry::ensure_registry_open(
        state,
        community_id,
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
    // One `SegmentDescriptor` now, so this is the value itself. The map
    // that used to be here rebuilt the struct field by field and
    // dropped `governance_key` on the way.
    crate::services::community::segments::segment_descriptors(state, community_id)
}

// ---- Per-community read shortcuts used by community_deps.rs ----

pub(super) fn my_pseudonym_for_community(state: &Arc<AppState>, community_id: &str) -> String {
    crate::state_helpers::my_pseudonym_key(state, community_id).unwrap_or_default()
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

/// Our slot keypair in `community_id`, derived from its slot seed and
/// cached on the community.
fn try_derive_slot_keypair(
    state: &Arc<AppState>,
    community_id: &str,
    seed_hex: &str,
    subkey_idx: u32,
) -> Option<String> {
    let seed_bytes = hex::decode(seed_hex).ok()?;
    let seed_array: [u8; 32] = seed_bytes.as_slice().try_into().ok()?;
    match derive::derive_slot_keypair(&seed_array, subkey_idx) {
        Ok(sk) => {
            let kp_str = rekindle_crypto::keypair_text::keypair_text_from_signing(&sk);
            {
                let mut communities = state.communities.write();
                if let Some(c) = communities.get_mut(community_id) {
                    c.slot_keypair = Some(kp_str.clone());
                }
            }
            Some(kp_str)
        }
        Err(e) => {
            tracing::warn!(error = %e, "failed to derive slot keypair from seed");
            None
        }
    }
}
