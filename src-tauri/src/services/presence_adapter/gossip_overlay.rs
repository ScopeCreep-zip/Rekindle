//! Thin AppState-bound helpers for `CommunityPresenceDeps`'s
//! gossip-overlay primitives. Owns the read/write side; the
//! rebuild DECISION lives in
//! `crates/rekindle-presence/src/community/overlay_rebuild.rs`
//! per Invariant 7.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use rekindle_presence::{GossipOverlayPlan, GossipOverlaySnapshot, OnlineMemberSnapshot};

use crate::channels::CommunityEvent;
use crate::state::{AppState, GossipOverlay, OnlineMember};
use crate::state_helpers;

pub(super) fn extend_online_with_recent_gossip(
    state: &Arc<AppState>,
    community_id: &str,
    online_members: &mut HashMap<String, OnlineMemberSnapshot>,
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
            online_members.insert(pk.clone(), online_member_from_state(member));
        }
    }
}

pub(super) fn gossip_offline_diff(
    state: &Arc<AppState>,
    community_id: &str,
    online_members: &HashMap<String, OnlineMemberSnapshot>,
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
    let peers: HashMap<String, OnlineMember> = plan
        .peers
        .into_iter()
        .map(|(k, v)| (k, state_online_from_snapshot(v)))
        .collect();
    let online_members: HashMap<String, OnlineMember> = plan
        .online_members
        .into_iter()
        .map(|(k, v)| (k, state_online_from_snapshot(v)))
        .collect();
    let remaining: VecDeque<_> = plan.remaining_pending;
    cs.gossip = Some(GossipOverlay {
        peers,
        online_members,
        lamport_counter: plan.lamport_counter,
        needs_initial_sync: plan.needs_initial_sync,
        pending_mesh_broadcasts: remaining,
    });
}

pub(super) fn emit_member_presence_offline(
    state: &Arc<AppState>,
    community_id: &str,
    pseudonym_key: &str,
) {
    if let Some(app_handle) = state_helpers::app_handle(state) {
        crate::event_dispatch::emit_live(
            &app_handle,
            "community-event",
            &CommunityEvent::MemberPresenceChanged {
                community_id: community_id.to_string(),
                pseudonym_key: pseudonym_key.to_string(),
                status: "offline".to_string(),
                game_name: None,
                game_id: None,
                elapsed_seconds: None,
                server_address: None,
            },
        );
    }
}

fn online_member_from_state(state_member: &OnlineMember) -> OnlineMemberSnapshot {
    OnlineMemberSnapshot {
        route_blob: state_member.route_blob.clone(),
        status: state_member.status.clone(),
        last_seen: state_member.last_seen,
    }
}

fn state_online_from_snapshot(snapshot: OnlineMemberSnapshot) -> OnlineMember {
    OnlineMember {
        route_blob: snapshot.route_blob,
        status: snapshot.status,
        last_seen: snapshot.last_seen,
    }
}
