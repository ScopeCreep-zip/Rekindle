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

    /// The fan-out targets for this community.
    ///
    /// `GossipMesh.peers` is the selected subset (size = fan-out
    /// degree); `online_members` is everyone reachable. The crate
    /// re-sorts and truncates to the degree itself, so handing it the
    /// full online set rather than the pre-selected subset lets its
    /// reliability weighting actually choose — with only `peers` it
    /// would be re-ranking a list something else already narrowed.
    pub(super) fn current_peers_impl(&self, community_id: &str) -> Option<Vec<PeerInfo>> {
        let guard = self.ctx.broadcast_mgr.read();
        let manager = guard.as_ref()?;
        let meshes = manager.meshes().read();
        let mesh = meshes.get(community_id)?;
        Some(
            mesh.online_members
                .iter()
                .filter(|(_, member)| !member.route_blob.is_empty())
                .map(|(pseudonym, member)| PeerInfo {
                    pseudonym_key: pseudonym.clone(),
                    route_blob: member.route_blob.clone(),
                })
                .collect(),
        )
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
