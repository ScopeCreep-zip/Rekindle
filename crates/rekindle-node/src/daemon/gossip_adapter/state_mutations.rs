//! Overlay mutations and delivery bookkeeping.

use rekindle_codec::community::envelope::SignedEnvelope;
use rekindle_transport::OnlineMember;

use super::DaemonGossipAdapter;

impl DaemonGossipAdapter {
    /// Dedup at the mesh layer.
    ///
    /// Delegates to the same `GossipMesh` the inbound path meters
    /// against, so a broadcast we originate cannot come back around the
    /// mesh and be re-processed as though a peer had sent it.
    pub(super) fn check_and_insert_dedup_impl(
        &self,
        community_id: &str,
        sender: &str,
        dedup_key: &str,
    ) {
        let guard = self.ctx.broadcast_mgr.read();
        let Some(manager) = guard.as_ref() else {
            return;
        };
        manager
            .mesh_dedup()
            .write()
            .check_and_insert(community_id, sender, dedup_key);
    }

    /// Hold a broadcast that found no peers (architecture A1/P4.1): it
    /// waits on the community's `pending_mesh_broadcasts`, oldest dropped
    /// past `MAX_PENDING_MESH`, and the next presence poll that finds
    /// online peers re-sends it (`rekindle_presence::community::poll`'s
    /// drain, as on the desktop). Gossip carries no durability, but it
    /// is not always redundant with PATH 1: an op whose `M`/2 append can
    /// never land (a restored backup's op at a seq the log already holds,
    /// D15) reaches the members only this way, and a node just unlocked
    /// has no peers until its first poll.
    pub(super) fn enqueue_pending_mesh_impl(&self, community_id: &str, signed: SignedEnvelope) {
        let mut map = self.ctx.communities.map().write();
        let Some(gossip) = map.get_mut(community_id).and_then(|c| c.gossip.as_mut()) else {
            tracing::debug!(
                community = %community_id,
                "gossip: no peers online and no overlay; broadcast not held"
            );
            return;
        };
        if gossip.pending_mesh_broadcasts.len() >= rekindle_gossip::MAX_PENDING_MESH {
            gossip.pending_mesh_broadcasts.pop_front();
        }
        gossip.pending_mesh_broadcasts.push_back(signed);
        tracing::debug!(
            community = %community_id,
            held = gossip.pending_mesh_broadcasts.len(),
            "gossip: no peers online, broadcast held for the next poll with peers"
        );
    }

    /// Patch a peer's route after a DHT re-resolve.
    ///
    /// Writes both `peers` and `online_members` so the next presence
    /// poll does not overwrite the fresh blob with the stale one it
    /// still has cached.
    pub(super) fn update_peer_route_impl(
        &self,
        community_id: &str,
        peer_key: &str,
        status: &str,
        route_blob: &[u8],
    ) {
        let guard = self.ctx.broadcast_mgr.read();
        let Some(manager) = guard.as_ref() else {
            return;
        };
        let mut meshes = manager.meshes().write();
        let Some(mesh) = meshes.get_mut(community_id) else {
            return;
        };
        let last_seen = self.ctx.clock.now_secs();
        for map in [&mut mesh.peers, &mut mesh.online_members] {
            map.insert(
                peer_key.to_string(),
                OnlineMember {
                    route_blob: route_blob.to_vec(),
                    status: status.to_string(),
                    last_seen,
                    // A gossip observation carries neither: `location`
                    // and `last_active` come off the member's signed
                    // registry row, which the presence poll decodes and
                    // `to_mesh_members` now carries through.
                    ..OnlineMember::default()
                },
            );
        }
    }
}
