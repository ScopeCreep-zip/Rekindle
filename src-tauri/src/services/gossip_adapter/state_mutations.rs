//! Gossip-overlay state mutations, extracted from `deps_impl.rs`.
//!
//! Each acquires a write lock, mutates, and drops the guard before any
//! `.await` — parking_lot guards are `!Send`.

use std::sync::Arc;

use rekindle_protocol::dht::community::envelope::SignedEnvelope;

use crate::state::{AppState, OnlineMember};

/// Advance the community's **gossip-mesh** Lamport clock.
///
/// NOT a duplicate of `state_helpers::increment_lamport` despite the
/// name: that helper advances the community CRDT clock
/// (`CommunityState::lamport_counter`); this one advances
/// `community.gossip.lamport_counter`. Different clocks — do not
/// "deduplicate" them onto one helper.
pub(super) fn increment_lamport(state: &Arc<AppState>, community_id: &str) {
    let mut communities = state.communities.write();
    if let Some(community) = communities.get_mut(community_id) {
        if let Some(ref mut gossip) = community.gossip {
            gossip.lamport_counter += 1;
        }
    }
}

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
/// Returns the bound voice transport when one is attached to this
/// community, so the caller can heal its roster entry outside the lock.
/// A successful re-resolve proves the peer's advertised voice route is
/// stale too, and frame sends have no re-resolve of their own.
#[must_use]
pub(super) fn update_peer_route(
    state: &Arc<AppState>,
    community_id: &str,
    peer_key: &str,
    status: &str,
    route_blob: Vec<u8>,
) -> Option<std::sync::Arc<tokio::sync::Mutex<rekindle_voice::transport::VoiceTransport>>> {
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

    let ve = state.voice_engine.lock();
    ve.as_ref()
        .filter(|h| h.community_id.as_deref() == Some(community_id))
        .map(|h| h.transport.clone())
}
