//! Phase 20 REDO — `GossipDeps` implementation for `GossipAdapter`.
//!
//! Maps each trait method to the live AppState / Db / Veilid
//! routing-context calls that pre-port lived directly inside
//! `services/community/gossip.rs`. Mutation paths drop locks before
//! awaiting (parking_lot guards are `!Send`).

use async_trait::async_trait;
use rekindle_codec::community::envelope::SignedEnvelope;
use rekindle_gossip::{GossipDeps, PeerInfo};

use crate::services::gossip_adapter::{state_mutations, state_reads, GossipAdapter};
use crate::state_helpers;

use std::collections::HashMap;

#[async_trait]
impl GossipDeps for GossipAdapter {
    fn my_pseudonym_key(&self, community_id: &str) -> String {
        state_helpers::my_pseudonym_key(&self.state, community_id).unwrap_or_default()
    }

    fn identity_secret(&self) -> Option<[u8; 32]> {
        state_helpers::identity_secret(&self.state)
    }

    fn scope(&self) -> std::sync::Arc<rekindle_lifecycle::SessionScope> {
        state_helpers::login_scope_or_closed(&self.state)
    }

    fn check_and_insert_dedup(&self, community_id: &str, sender: &str, dedup_key: &str) {
        self.state
            .dedup_cache
            .lock()
            .check_and_insert(community_id, sender, dedup_key);
    }

    fn current_peers(&self, community_id: &str) -> Option<Vec<PeerInfo>> {
        state_reads::current_peers(&self.state, community_id)
    }

    fn peer_reliability_scores(&self, community_id: &str) -> HashMap<String, f64> {
        state_reads::peer_reliability_scores(&self.state, community_id)
    }

    fn online_member_status(&self, community_id: &str, peer_key: &str) -> Option<String> {
        state_reads::online_member_status(&self.state, community_id, peer_key)
    }

    fn enqueue_pending_mesh(&self, community_id: &str, signed: SignedEnvelope) {
        state_mutations::enqueue_pending_mesh(&self.state, community_id, signed);
    }

    fn update_peer_route(
        &self,
        community_id: &str,
        peer_key: &str,
        status: &str,
        route_blob: Vec<u8>,
    ) {
        state_mutations::update_peer_route(&self.state, community_id, peer_key, status, route_blob);
    }

    fn record_peer_reliability(&self, community_id: &str, peer_key: &str, success: bool) {
        // Delegate to the standalone src-tauri wrapper so the
        // in-memory mutation + dirty-set flag live in one place
        // (architecture §14.5). The wrapper is also exposed for
        // out-of-mesh callers (sync_service retry path, voice
        // signaling failure paths) per the gossip plan.
        crate::services::community::record_peer_reliability(
            &self.state,
            community_id,
            peer_key,
            success,
        );
    }

    async fn record_delivery(
        &self,
        message_id: &str,
        community_id: &str,
        recipient: &str,
        status: &str,
    ) {
        let mid = message_id.to_string();
        let cid = community_id.to_string();
        let rp = recipient.to_string();
        let st = status.to_string();
        let now = rekindle_utils::timestamp_secs();
        crate::db_helpers::db_fire(&self.pool, "record_delivery", move |conn| {
            conn.execute(
                "INSERT INTO message_delivery (message_id, community_id, recipient_pseudonym, status, attempts, last_attempt_at) \
                 VALUES (?1, ?2, ?3, ?4, 1, ?5) \
                 ON CONFLICT(message_id, recipient_pseudonym) \
                 DO UPDATE SET status=excluded.status, attempts=attempts+1, last_attempt_at=excluded.last_attempt_at",
                rusqlite::params![mid, cid, rp, st, now.cast_signed()],
            )?;
            Ok(())
        });
    }

    async fn resolve_peer_route_from_dht(
        &self,
        community_id: &str,
        peer_pseudonym: &str,
    ) -> Option<Vec<u8>> {
        crate::services::community::routes::resolve_member_route(
            &self.state,
            &self.pool,
            community_id,
            peer_pseudonym,
        )
        .await
    }

    fn resolve_gate(&self) -> &rekindle_gossip::ResolveGate {
        &self.state.gossip_resolve_gate
    }

    async fn send_app_message(&self, route_blob: &[u8], data: Vec<u8>) -> Result<(), String> {
        state_helpers::message_route_blob(&self.state, route_blob, data).await
    }
}

/// Build a one-shot adapter from the live AppState. The facade in
/// `services/community/gossip.rs` constructs this per call (cheap —
/// just clones two Arcs) and hands it to the crate's orchestrators.
pub fn build_adapter(state: &std::sync::Arc<crate::state::AppState>) -> Option<GossipAdapter> {
    // GossipAdapter needs only state + pool; the handle is dropped.
    let (_app_handle, pool) = state_helpers::app_context(state)?;
    Some(GossipAdapter::new(std::sync::Arc::clone(state), pool))
}
