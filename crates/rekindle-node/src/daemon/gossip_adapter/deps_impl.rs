//! The single `GossipDeps` impl for the daemon. Delegation only.

use std::collections::HashMap;

use async_trait::async_trait;
use rekindle_codec::community::envelope::SignedEnvelope;
use rekindle_gossip::resolve_gate::ResolveGate;
use rekindle_gossip::{GossipDeps, PeerInfo};

use super::DaemonGossipAdapter;

#[async_trait]
impl GossipDeps for DaemonGossipAdapter {
    // ---------- Identity ----------

    fn my_pseudonym_key(&self, community_id: &str) -> String {
        self.my_pseudonym_key_impl(community_id)
    }

    fn identity_secret(&self) -> Option<[u8; 32]> {
        self.identity_secret_impl()
    }

    fn scope(&self) -> std::sync::Arc<rekindle_lifecycle::SessionScope> {
        self.ctx.unlock_scope_or_closed()
    }

    fn rng(&self) -> std::sync::Arc<dyn rekindle_utils::RandomSource> {
        std::sync::Arc::clone(&self.ctx.rng)
    }

    // ---------- Mesh bookkeeping ----------

    fn check_and_insert_dedup(&self, community_id: &str, sender: &str, dedup_key: &str) {
        self.check_and_insert_dedup_impl(community_id, sender, dedup_key);
    }

    // ---------- Overlay reads ----------

    fn current_peers(&self, community_id: &str) -> Option<Vec<PeerInfo>> {
        Some(self.current_peers_impl(community_id))
    }

    fn peer_reliability_scores(&self, community_id: &str) -> HashMap<String, f64> {
        Self::peer_reliability_scores_impl(community_id)
    }

    fn online_member_status(&self, community_id: &str, peer_key: &str) -> Option<String> {
        self.online_member_status_impl(community_id, peer_key)
    }

    // ---------- Overlay mutations ----------

    fn enqueue_pending_mesh(&self, community_id: &str, signed: SignedEnvelope) {
        self.enqueue_pending_mesh_impl(community_id, signed);
    }

    fn update_peer_route(
        &self,
        community_id: &str,
        peer_key: &str,
        status: &str,
        route_blob: Vec<u8>,
    ) {
        self.update_peer_route_impl(community_id, peer_key, status, &route_blob);
    }

    /// Per-peer delivery counters have nowhere durable to live on this
    /// track — the daemon has no SQLite. See `peer_reliability_scores`:
    /// recording into memory that nothing reads would be worse than not
    /// recording, because it would look like the feature works.
    fn record_peer_reliability(&self, _community_id: &str, _peer_key: &str, _success: bool) {}

    // ---------- Route resolution ----------

    /// A send to the peer's route failed: the Host forgets its cached
    /// route and re-reads the peer's `M`/6 once, pinned to its
    /// `MemberJoined` owner and verified under its pseudonym (plan E3.4
    /// S4).
    async fn resolve_peer_route_from_dht(
        &self,
        community_id: &str,
        peer_pseudonym: &str,
    ) -> Option<Vec<u8>> {
        crate::host::adapters::governance::HostGovernance::new(std::sync::Arc::clone(&self.ctx))
            .reread_member_route(community_id, peer_pseudonym)
            .await
    }

    fn resolve_gate(&self) -> &ResolveGate {
        &self.resolve_gate
    }

    // ---------- Transport ----------

    async fn send_app_message(&self, route_blob: &[u8], data: Vec<u8>) -> Result<(), String> {
        let node = self.transport().ok_or_else(|| "not attached".to_string())?;
        node.caller()
            .send_unframed_to_route(route_blob, data)
            .await
            .map_err(|e| e.to_string())
    }
}
