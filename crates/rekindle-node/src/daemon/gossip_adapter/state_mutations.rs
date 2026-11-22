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
        // The shell meters its inbound gossip against its own cache until
        // the inbound path moves into the Host.
        if let Some(shell) = &self.ctx.shell {
            shell.own_gossip_signed(community_id, sender, dedup_key);
            return;
        }
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
        let Some(community) = map.get_mut(community_id) else {
            tracing::debug!(
                community = %community_id,
                "gossip: broadcast for a community we do not hold; not held"
            );
            return;
        };
        let gossip = community
            .gossip
            .get_or_insert_with(crate::host::community::GossipOverlay::default);
        if gossip.pending_mesh_broadcasts.len() >= rekindle_gossip::MAX_PENDING_MESH {
            gossip.pending_mesh_broadcasts.pop_front();
        }
        gossip.pending_mesh_broadcasts.push_back(signed);
        tracing::debug!(
            community = %community_id,
            held = gossip.pending_mesh_broadcasts.len(),
            "gossip: the mesh is empty, broadcast held until a member's beat goes live"
        );
    }

    /// Patch a peer's route after a DHT re-resolve.
    ///
    /// Writes `online_members`, the set heard from; the fan-out reads the
    /// Host's mesh (plan E3.4 S5), never this set.
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
        mesh.online_members.insert(
            peer_key.to_string(),
            OnlineMember {
                route_blob: route_blob.to_vec(),
                status: status.to_string(),
                last_seen,
                // A gossip observation carries neither: `location` and
                // `last_active` come off the member's signed registry row.
                ..OnlineMember::default()
            },
        );
    }
}
