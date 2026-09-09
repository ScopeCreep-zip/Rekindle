//! Thin AppState-bound helpers for `CommunityPresenceDeps`'s
//! gossip-overlay primitives. Owns the read/write side; the
//! rebuild DECISION lives in
//! `crates/rekindle-presence/src/community/overlay_rebuild.rs`
//! per Invariant 7.

use std::collections::HashMap;
use std::sync::Arc;

use rekindle_presence::{GossipOverlayPlan, GossipOverlaySnapshot, OnlineMember};

use crate::state::{AppState, GossipOverlay};
use crate::state_helpers;

pub(super) fn extend_online_with_recent_gossip(
    state: &Arc<AppState>,
    community_id: &str,
    online_members: &mut HashMap<String, OnlineMember>,
    my_pseudonym: &str,
    eviction_threshold_secs: u64,
) {
    let now = rekindle_utils::timestamp_secs();
    let eviction_cutoff = now.saturating_sub(eviction_threshold_secs);
    let communities = state.communities.read();
    let Some(cs) = communities.get(community_id) else {
        return;
    };
    let Some(ref gossip) = cs.gossip else {
        return;
    };
    for (pk, member) in &gossip.online_members {
        if !online_members.contains_key(pk)
            && pk != my_pseudonym
            && member.last_seen > eviction_cutoff
        {
            online_members.insert(pk.clone(), member.clone());
        }
    }
}

pub(super) fn gossip_offline_diff(
    state: &Arc<AppState>,
    community_id: &str,
    online_members: &HashMap<String, OnlineMember>,
    my_pseudonym: &str,
) -> Vec<String> {
    let communities = state.communities.read();
    communities
        .get(community_id)
        .and_then(|cs| cs.gossip.as_ref())
        .map(|gossip| {
            gossip
                .online_members
                .keys()
                .filter(|pk| !online_members.contains_key(*pk) && *pk != my_pseudonym)
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

pub(super) fn read_gossip_snapshot(
    state: &Arc<AppState>,
    community_id: &str,
) -> GossipOverlaySnapshot {
    let mut communities = state.communities.write();
    let Some(cs) = communities.get_mut(community_id) else {
        return GossipOverlaySnapshot::default();
    };
    let lamport_counter = cs.gossip.as_ref().map_or(0, |g| g.lamport_counter);
    let needs_initial_sync = cs.gossip.as_ref().is_none_or(|g| g.needs_initial_sync);
    // Drain the pending queue here under the write lock — pre-port
    // poll.rs did the same `std::mem::take` so the queue doesn't
    // double-fire on the next tick. The crate's planner decides
    // whether to actually send (when peers become non-empty) or
    // restore (when still empty).
    let pending = cs
        .gossip
        .as_mut()
        .map(|g| std::mem::take(&mut g.pending_mesh_broadcasts))
        .unwrap_or_default();
    GossipOverlaySnapshot {
        lamport_counter,
        needs_initial_sync,
        pending_mesh_broadcasts: pending,
    }
}

pub(super) fn apply_gossip_rebuild_plan(
    state: &Arc<AppState>,
    community_id: &str,
    plan: GossipOverlayPlan,
) {
    let mut communities = state.communities.write();
    let Some(cs) = communities.get_mut(community_id) else {
        return;
    };
    // The plan's maps are already the state's maps: one `OnlineMember`
    // type now, so there is nothing to convert between them.
    cs.gossip = Some(GossipOverlay {
        peers: plan.peers,
        online_members: plan.online_members,
        lamport_counter: plan.lamport_counter,
        needs_initial_sync: plan.needs_initial_sync,
        pending_mesh_broadcasts: plan.remaining_pending,
    });
}

pub(super) fn emit_member_presence_offline(
    state: &Arc<AppState>,
    community_id: &str,
    pseudonym_key: &str,
) {
    if let Some(app_handle) = state_helpers::app_handle(state) {
        crate::event_dispatch::emit_subscription(
            &app_handle,
            &rekindle_types::subscription_events::SubscriptionEvent::Presence(
                rekindle_types::subscription_events::PresenceEvent::CommunityMemberChanged {
                    community: community_id.to_string(),
                    pseudonym: pseudonym_key.to_string(),
                    // Status only: the overlay timed the member out, it
                    // did not look at what they were playing. Leaving
                    // `game` unobserved is what stops this from
                    // clearing a game a gossip row had set.
                    snapshot: rekindle_types::subscription_events::PresenceSnapshot::status(
                        "offline",
                    ),
                },
            ),
        );
    }
}
