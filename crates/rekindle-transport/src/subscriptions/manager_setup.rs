//! Watch setup / teardown for identities, communities, and DM peers.

use tracing::{debug, info};

use super::{watches, SubscriptionManager};
use crate::gossip::GossipMesh;
use crate::session::{CommunityMembership, Session};

impl SubscriptionManager {
    /// Establish all identity-level watches (friend inbox, DM logs).
    ///
    /// Called once after identity is loaded and session is resumed.
    /// Idempotent — safe to call multiple times.
    pub async fn setup_identity(&self, session: &Session) {
        watches::setup_identity_watches(&self.node, &self.watches, session).await;
        info!(
            friend_inbox = %session.identity.friend_inbox_key,
            dm_peers = session.dm_log_keys.len(),
            "identity watches established"
        );
    }

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

    /// Remove all watches and state for a community.
    pub fn teardown_community(&self, governance_key: &str) {
        self.watches.write().remove_community(governance_key);
        self.meshes.write().remove(governance_key);
        self.state.write().unread.remove_community(governance_key);
        self.state.write().typing.remove_community(governance_key);
        self.state.write().presence.remove_community(governance_key);
        self.state.write().voice.remove_community(governance_key);
        debug!(governance_key, "community subscriptions torn down");
    }

    /// Set up a DM peer watch.
    pub async fn setup_dm_peer(&self, peer_key: &str, dm_log_key: &str) {
        debug!(
            peer = &peer_key[..12.min(peer_key.len())],
            dm_log_key, "sub: setup_dm_peer"
        );
        watches::setup_dm_watch(&self.node, &self.watches, peer_key, dm_log_key).await;
    }

    /// Remove DM watch and state for a peer.
    pub fn teardown_dm_peer(&self, peer_key: &str) {
        debug!(
            peer = &peer_key[..12.min(peer_key.len())],
            "sub: teardown_dm_peer"
        );
        self.watches.write().remove_dm_peer(peer_key);
        self.state.write().unread.remove_dm_peer(peer_key);
        self.state.write().typing.remove_dm_peer(peer_key);
        self.state.write().presence.remove_dm_peer(peer_key);
    }
}
