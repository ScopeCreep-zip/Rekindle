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
