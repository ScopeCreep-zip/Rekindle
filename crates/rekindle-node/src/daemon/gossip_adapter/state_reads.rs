//! Overlay and identity reads.

use std::collections::HashMap;

use rekindle_gossip::PeerInfo;

use super::DaemonGossipAdapter;

impl DaemonGossipAdapter {
    pub(super) fn my_pseudonym_key_impl(&self, community_id: &str) -> String {
        self.ctx.my_pseudonym(community_id)
    }

    pub(super) fn identity_secret_impl(&self) -> Option<[u8; 32]> {
        self.ctx.identity_secret()
    }

    /// The fan-out targets for this community: the Host's mesh (plan E3.4
    /// S5), the roster's live members by their `M`/6 routes, already
    /// picked; the crate's degree keeps them all and, with no scores here,
    /// its stable sort keeps their order.
    pub(super) fn current_peers_impl(&self, community_id: &str) -> Vec<PeerInfo> {
        crate::host::adapters::governance::HostGovernance::new(std::sync::Arc::clone(&self.ctx))
            .mesh_peers(community_id)
    }

    /// Per-peer delivery reliability (§14.5, gossip topology
    /// optimization).
    ///
    /// The daemon keeps no delivery history — it has no SQLite, and
    /// `record_peer_reliability` has nowhere durable to write. Returning
    /// an empty map is honest: the crate's `sort_peers_by_reliability`
    /// treats an absent score as neutral, so fan-out selection stays
    /// uniform rather than being skewed by fabricated numbers. The
    /// "ziplines" the architecture describes emerge on the desktop
    /// track, which does have the store.
    pub(super) fn peer_reliability_scores_impl(_community_id: &str) -> HashMap<String, f64> {
        HashMap::new()
    }

    pub(super) fn online_member_status_impl(
        &self,
        community_id: &str,
        peer_key: &str,
    ) -> Option<String> {
        let guard = self.ctx.broadcast_mgr.read();
        let manager = guard.as_ref()?;
        let meshes = manager.meshes().read();
        meshes
            .get(community_id)?
            .online_members
            .get(peer_key)
            .map(|member| member.status.clone())
    }
}
