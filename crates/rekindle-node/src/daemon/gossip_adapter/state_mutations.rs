//! Overlay mutations and delivery bookkeeping.

use rekindle_protocol::dht::community::envelope::SignedEnvelope;
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

    pub(super) fn increment_lamport_impl(&self, community_id: &str) {
        let guard = self.ctx.broadcast_mgr.read();
        let Some(manager) = guard.as_ref() else {
            return;
        };
        let mut meshes = manager.meshes().write();
        if let Some(mesh) = meshes.get_mut(community_id) {
            mesh.clock.increment();
        }
    }

    /// Hold a broadcast that found no peers.
    ///
    /// The daemon has no pending-mesh queue. Dropping is the correct
    /// outcome rather than a shortfall: gossip is PATH 2, explicitly
    /// "Durability: None" in the architecture, and the SMPL write on
    /// PATH 1 already carries the content. A peer that was offline for
    /// this wave picks it up from the record via PATH 3. Queueing would
    /// re-deliver a notification for something the recipient will have
    /// already read.
    pub(super) fn enqueue_pending_mesh_impl(community_id: &str, signed: &SignedEnvelope) {
        tracing::debug!(
            community = %community_id,
            sender = %&signed.sender_pseudonym[..12.min(signed.sender_pseudonym.len())],
            "gossip: no peers online, dropping broadcast (PATH 1 write is authoritative)"
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
        let last_seen = rekindle_utils::timestamp_secs();
        for map in [&mut mesh.peers, &mut mesh.online_members] {
            map.insert(
                peer_key.to_string(),
                OnlineMember {
                    route_blob: route_blob.to_vec(),
                    status: status.to_string(),
                    last_seen,
                },
            );
        }
    }
}
