//! Watch setup / teardown for communities.

use crate::broadcast::dht_writes::LeaseId;
use tracing::{debug, info};

use super::{watches, SubscriptionManager};
use crate::gossip::GossipMesh;
use crate::session::CommunityMembership;

impl SubscriptionManager {
    /// Establish all community-level watches (governance, registry, inbox).
    ///
    /// Called for each community during resume and on community join.
    pub async fn setup_community(&self, membership: &CommunityMembership) {
        info!(community = %membership.community_name, governance = %membership.governance_key, "sub: setup_community");
        watches::setup_community_watches(&self.node, &self.watches, membership).await;
        self.meshes
            .write()
            .entry(membership.governance_key.clone())
            .or_insert_with(|| GossipMesh::new(membership.governance_key.clone()));
    }

    /// Do we hold a watch on this record?
    ///
    /// Used by the Mutual Aid watch relay (§14.3) to skip a redundant
    /// fetch: if our own watch covers the record, its value-change
    /// callback reports the same change a peer is relaying to us.
    #[must_use]
    pub fn has_watch(&self, record_key: &str) -> bool {
        self.watches.read().entries.contains_key(record_key)
    }

    /// Remove all watches and state for a community. Returns the watches'
    /// pool leases, for the caller to release.
    #[must_use]
    pub fn teardown_community(&self, governance_key: &str) -> Vec<LeaseId> {
        let leases = self.watches.write().remove_community(governance_key);
        self.meshes.write().remove(governance_key);
        self.state.write().unread.remove_community(governance_key);
        self.state.write().typing.remove_community(governance_key);
        self.state.write().presence.remove_community(governance_key);
        self.state.write().voice.remove_community(governance_key);
        debug!(governance_key, "community subscriptions torn down");
        leases
    }
}
