//! Gossip-overlay state reads, extracted from `deps_impl.rs`.
//!
//! Mirrors the layout the other adapters already use
//! (`governance_adapter`, `channel_adapter`): the trait impl stays a
//! list of one-line delegations, and the bodies that reach into
//! `AppState` live beside it. This adapter was the only one that had
//! never been split, so its `GossipDeps` impl carried six inline
//! `communities.read()/.write()` bodies.

use std::collections::HashMap;
use std::sync::Arc;

use rekindle_gossip::PeerInfo;

use crate::state::AppState;

/// Peers currently in the community's gossip overlay.
pub(super) fn current_peers(state: &Arc<AppState>, community_id: &str) -> Option<Vec<PeerInfo>> {
    let communities = state.communities.read();
    let gossip = communities.get(community_id)?.gossip.as_ref()?;
    Some(
        gossip
            .peers
            .iter()
            .map(|(key, member)| PeerInfo {
                pseudonym_key: key.clone(),
                route_blob: member.route_blob.clone(),
            })
            .collect(),
    )
}

/// Delivery-success scores per peer, derived from the raw counters.
pub(super) fn peer_reliability_scores(
    state: &Arc<AppState>,
    community_id: &str,
) -> HashMap<String, f64> {
    let communities = state.communities.read();
    let Some(community) = communities.get(community_id) else {
        return HashMap::new();
    };
    rekindle_gossip::scores_from_counters(&community.peer_reliability)
}

/// A peer's last-known presence status, if we have seen them online.
pub(super) fn online_member_status(
    state: &Arc<AppState>,
    community_id: &str,
    peer_key: &str,
) -> Option<String> {
    state
        .communities
        .read()
        .get(community_id)?
        .gossip
        .as_ref()?
        .online_members
        .get(peer_key)
        .map(|m| m.status.clone())
}
