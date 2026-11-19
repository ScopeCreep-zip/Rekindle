//! Community setup / teardown: the gossip mesh and the subscription
//! state. No record is watched here: the per-channel-record watch and the
//! tier-3 poll went with the channel slot record (plan E3.3 slice 8); a
//! member's channel outbox is watched by the Host's ring (`M`/4).

use tracing::{debug, info};

use super::SubscriptionManager;
use crate::gossip::GossipMesh;
use crate::session::CommunityMembership;

impl SubscriptionManager {
    /// Set up a community's gossip mesh.
    ///
    /// Called for each community during resume and on community join.
    pub async fn setup_community(&self, membership: &CommunityMembership) {
        info!(community = %membership.community_name, governance = %membership.anchor, "sub: setup_community");
        // The mesh first: it holds the community's message Lamport clock, so
        // a send works at once.
        self.meshes
            .write()
            .entry(membership.anchor.clone())
            .or_insert_with(|| GossipMesh::new(membership.anchor.clone()));
    }

    /// Remove all state for a community.
    pub fn teardown_community(&self, anchor: &str) {
        self.meshes.write().remove(anchor);
        self.state.write().typing.remove_community(anchor);
        self.state.write().presence.remove_community(anchor);
        self.state.write().voice.remove_community(anchor);
        debug!(anchor, "community subscriptions torn down");
    }
}
