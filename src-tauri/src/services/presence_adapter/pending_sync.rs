//! Pending-sync retry helpers for the community-presence adapter.
//! Owns `stale_pending_syncs`, `update_pending_sync`,
//! `prune_pending_syncs` — the per-tick stale-SyncRequest sweep.

use std::sync::Arc;

use crate::state::AppState;

pub(super) fn stale_pending_syncs(
    state: &Arc<AppState>,
    community_id: &str,
    now_secs: u64,
    stale_window_secs: u64,
    max_attempts: u32,
) -> Vec<(String, u32)> {
    let communities = state.communities.read();
    communities.get(community_id).map_or(Vec::new(), |cs| {
        cs.pending_syncs
            .iter()
            .filter(|(_, (ts, count))| {
                now_secs.saturating_sub(*ts) > stale_window_secs && *count < max_attempts
            })
            .map(|(ch, (_, count))| (ch.clone(), *count))
            .collect()
    })
}

pub(super) fn update_pending_sync(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
    now_secs: u64,
    attempt: u32,
) {
    let mut communities = state.communities.write();
    if let Some(cs) = communities.get_mut(community_id) {
        cs.pending_syncs
            .insert(channel_id.to_string(), (now_secs, attempt));
    }
}

pub(super) fn prune_pending_syncs(state: &Arc<AppState>, community_id: &str, max_attempts: u32) {
    let mut communities = state.communities.write();
    if let Some(cs) = communities.get_mut(community_id) {
        cs.pending_syncs
            .retain(|_, (_, count)| *count < max_attempts);
    }
}
