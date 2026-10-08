//! Phase 23.C — community-unread bodies lifted from
//! `commands/community/unread.rs`. Tauri commands stay thin
//! delegations; this file hosts the AppState read + mutate pair.
//! Pure state access — no protocol logic per Invariant 7.

use crate::state::SharedState;

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnreadCountEntry {
    pub channel_id: String,
    pub unread_count: u32,
}

pub fn mark_channel_read_inner(state: &SharedState, community_id: &str, channel_id: &str) {
    let mut communities = state.communities.write();
    if let Some(community) = communities.get_mut(community_id) {
        if let Some(ch) = community.channels.iter_mut().find(|c| c.id == channel_id) {
            ch.unread_count = 0;
        }
    }
}

pub fn get_unread_counts_inner(
    state: &SharedState,
    community_id: &str,
) -> Result<Vec<UnreadCountEntry>, String> {
    let _g =
        rekindle_lifecycle::TransportGuard::read(&state.lifecycle).map_err(|e| e.to_string())?;
    let communities = state.communities.read();
    let community = communities.get(community_id).ok_or("community not found")?;
    Ok(community
        .channels
        .iter()
        .map(|ch| UnreadCountEntry {
            channel_id: ch.id.clone(),
            unread_count: ch.unread_count,
        })
        .collect())
}
