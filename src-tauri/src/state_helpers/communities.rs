//! Community channel-list write helpers + governance-key collection.

use std::sync::Arc;

use crate::state::AppState;

/// Replace the entire channel list for a community.
pub fn set_community_channels(
    state: &Arc<AppState>,
    community_id: &str,
    channels: Vec<crate::state::ChannelInfo>,
) {
    let mut communities = state.communities.write();
    if let Some(community) = communities.get_mut(community_id) {
        community.channels = channels;
    }
}

/// Append a single channel to a community's channel list.
pub fn push_community_channel(
    state: &Arc<AppState>,
    community_id: &str,
    channel: crate::state::ChannelInfo,
) {
    let mut communities = state.communities.write();
    if let Some(community) = communities.get_mut(community_id) {
        community.channels.push(channel);
    }
}

/// Collect communities with governance record keys.
pub fn communities_with_governance_keys(state: &Arc<AppState>) -> Vec<(String, String)> {
    state
        .communities
        .read()
        .values()
        .filter_map(|c| c.governance_key.as_ref().map(|k| (c.id.clone(), k.clone())))
        .collect()
}

/// The MEK hierarchy for CHANNEL MEDIA (voice frames, video frames):
/// the per-channel MEK when the §10.5 join/leave rotation has
/// distributed one, otherwise the community MEK every member holds
/// from join. This mirrors the text plane's
/// `channel_or_community_mek_impl` — one key hierarchy for every
/// channel payload. Stage channels never rotate (§10.7), so they
/// resolve to the community MEK by construction. Returns
/// `(key_bytes, generation)`.
pub fn channel_media_mek(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
) -> Option<([u8; 32], u64)> {
    let channel = state
        .channel_mek_cache
        .lock()
        .get(&(community_id.to_string(), channel_id.to_string()))
        .map(|m| (*m.as_bytes(), m.generation()));
    channel.or_else(|| {
        state
            .mek_cache
            .lock()
            .get(community_id)
            .map(|m| (*m.as_bytes(), m.generation()))
    })
}
