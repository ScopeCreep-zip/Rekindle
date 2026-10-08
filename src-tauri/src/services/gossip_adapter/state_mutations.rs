//! Gossip-overlay state mutations, extracted from `deps_impl.rs`.
//!
//! Each acquires a write lock, mutates, and drops the guard before any
//! `.await` — parking_lot guards are `!Send`.

use std::sync::Arc;

use rekindle_codec::community::envelope::SignedEnvelope;

use crate::state::{AppState, OnlineMember};

/// Queue an envelope for a mesh broadcast that could not go out yet,
/// evicting the oldest once the queue is full.
pub(super) fn enqueue_pending_mesh(
    state: &Arc<AppState>,
    community_id: &str,
    signed: SignedEnvelope,
) {
    let mut communities = state.communities.write();
    let Some(community) = communities.get_mut(community_id) else {
        return;
    };
    let Some(ref mut gossip) = community.gossip else {
        return;
    };
    if gossip.pending_mesh_broadcasts.len() >= rekindle_gossip::MAX_PENDING_MESH {
        gossip.pending_mesh_broadcasts.pop_front();
    }
    gossip.pending_mesh_broadcasts.push_back(signed);
}

/// Record a freshly re-resolved route for a peer in the gossip overlay.
///
/// The route is the peer's general route, from its presence row. It never
/// reaches the voice transport: a call's media route travels only in voice
/// signaling, and the owner re-announces it when it changes (plan C7.15,
/// C7.23).
pub(super) fn update_peer_route(
    state: &Arc<AppState>,
    community_id: &str,
    peer_key: &str,
    status: &str,
    route_blob: Vec<u8>,
) {
    {
        let mut communities = state.communities.write();
        if let Some(community) = communities.get_mut(community_id) {
            if let Some(ref mut gossip) = community.gossip {
                let member = OnlineMember {
                    route_blob,
                    status: status.to_string(),
                    last_seen: rekindle_utils::timestamp_secs(),
                    ..Default::default()
                };
                gossip
                    .online_members
                    .insert(peer_key.to_string(), member.clone());
                // Refresh-only: never promote an online member into the
                // media plane's peer set.
                if gossip.peers.contains_key(peer_key) {
                    gossip.peers.insert(peer_key.to_string(), member);
                }
            }
        }
    }
}
