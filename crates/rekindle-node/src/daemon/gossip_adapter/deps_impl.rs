//! The single `GossipDeps` impl for the daemon. Delegation only.

use std::collections::HashMap;

use async_trait::async_trait;
use rekindle_gossip::resolve_gate::ResolveGate;
use rekindle_gossip::{GossipDeps, PeerInfo};
use rekindle_protocol::dht::community::envelope::SignedEnvelope;

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

    // ---------- Mesh bookkeeping ----------

    fn check_and_insert_dedup(&self, community_id: &str, sender: &str, dedup_key: &str) {
        self.check_and_insert_dedup_impl(community_id, sender, dedup_key);
    }

    fn increment_lamport(&self, community_id: &str) {
        self.increment_lamport_impl(community_id);
    }

    // ---------- Overlay reads ----------

    fn current_peers(&self, community_id: &str) -> Option<Vec<PeerInfo>> {
        self.current_peers_impl(community_id)
    }

    fn peer_reliability_scores(&self, community_id: &str) -> HashMap<String, f64> {
        Self::peer_reliability_scores_impl(community_id)
    }

    fn online_member_status(&self, community_id: &str, peer_key: &str) -> Option<String> {
        self.online_member_status_impl(community_id, peer_key)
    }

    // ---------- Overlay mutations ----------

    fn enqueue_pending_mesh(&self, community_id: &str, signed: SignedEnvelope) {
        Self::enqueue_pending_mesh_impl(community_id, &signed);
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

    /// Same store, same absence. The desktop writes a `message_delivery`
    /// row per attempt; the daemon has no table to write it to, and
    /// gossip is best-effort by design (PATH 2, "Durability: None"), so
    /// the delivery record is an observability nicety rather than part
    /// of the protocol.
    async fn record_delivery(
        &self,
        _message_id: &str,
        _community_id: &str,
        _recipient: &str,
        _status: &str,
    ) {
    }

    // ---------- Route resolution ----------

    /// Re-read a peer's route blob from their registry presence row.
    ///
    /// The roster the presence poll materialised is the source: every
    /// row behind it was W26-verified when it was scanned, so a route
    /// taken from here is one the named pseudonym actually signed.
    async fn resolve_peer_route_from_dht(
        &self,
        community_id: &str,
        peer_pseudonym: &str,
    ) -> Option<Vec<u8>> {
        self.resolve_peer_route_impl(community_id, peer_pseudonym)
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
