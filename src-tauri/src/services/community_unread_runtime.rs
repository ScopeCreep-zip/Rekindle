//! Phase 23.C — community-unread bodies lifted from
//! `commands/community/unread.rs`. Tauri commands stay thin
//! delegations; this file hosts the AppState read + mutate pair.
//! Pure state access — no protocol logic per Invariant 7.

use rekindle_types::subscription_events::{SubscriptionEvent, UnreadContext};

use crate::state::{AppState, SharedState};

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnreadCountEntry {
    pub channel_id: String,
    pub unread_count: u32,
}

pub fn mark_channel_read_inner(state: &SharedState, community_id: &str, channel_id: &str) {
    if set_channel_unread(state, community_id, channel_id, |_| 0).is_some() {
        publish_channel_unread(state, community_id, channel_id, 0);
    }
}

/// Count one more unread message stored in a channel and tell the
/// frontends the count, which they render as is (the DM twin is the
/// transport's `dm::chat::count_unread`).
pub fn count_channel_unread(state: &AppState, community_id: &str, channel_id: &str) {
    if let Some(count) =
        set_channel_unread(state, community_id, channel_id, |n| n.saturating_add(1))
    {
        publish_channel_unread(state, community_id, channel_id, count);
    }
}

fn set_channel_unread(
    state: &AppState,
    community_id: &str,
    channel_id: &str,
    next: impl FnOnce(u32) -> u32,
) -> Option<u32> {
    let mut communities = state.communities.write();
    let ch = communities
        .get_mut(community_id)?
        .channels
        .iter_mut()
        .find(|c| c.id == channel_id)?;
    ch.unread_count = next(ch.unread_count);
    Some(ch.unread_count)
}

fn publish_channel_unread(state: &AppState, community_id: &str, channel_id: &str, count: u32) {
    state.host.publish(SubscriptionEvent::UnreadChanged {
        context: UnreadContext::Channel {
            community: community_id.to_string(),
            channel: channel_id.to_string(),
        },
        count,
    });
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
