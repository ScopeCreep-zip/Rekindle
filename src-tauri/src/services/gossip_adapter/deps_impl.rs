//! Phase 20 REDO — `GossipDeps` implementation for `GossipAdapter`.
//!
//! Maps each trait method to the live AppState / DbPool / Veilid
//! routing-context calls that pre-port lived directly inside
//! `services/community/gossip.rs`. Mutation paths drop locks before
//! awaiting (parking_lot guards are `!Send`).

use async_trait::async_trait;
use rekindle_gossip::{GossipDeps, PeerInfo};
use rekindle_protocol::dht::community::envelope::SignedEnvelope;
use rekindle_protocol::dht::community::member_registry;
use rekindle_protocol::dht::DHTManager;
use tauri::Manager as _;

use crate::services::gossip_adapter::GossipAdapter;
use crate::state::OnlineMember;
use crate::state_helpers;

use std::collections::HashMap;

#[async_trait]
impl GossipDeps for GossipAdapter {
    fn my_pseudonym_key(&self, community_id: &str) -> String {
        let communities = self.state.communities.read();
        communities
            .get(community_id)
            .and_then(|c| c.my_pseudonym_key.clone())
            .unwrap_or_default()
    }

    fn identity_secret(&self) -> Option<[u8; 32]> {
        *self.state.identity_secret.lock()
    }

    fn check_and_insert_dedup(&self, community_id: &str, sender: &str, dedup_key: &str) {
        self.state
            .dedup_cache
            .lock()
            .check_and_insert(community_id, sender, dedup_key);
    }

    fn increment_lamport(&self, community_id: &str) {
        let mut communities = self.state.communities.write();
        if let Some(community) = communities.get_mut(community_id) {
            if let Some(ref mut gossip) = community.gossip {
                gossip.lamport_counter += 1;
            }
        }
    }

    fn current_peers(&self, community_id: &str) -> Option<Vec<PeerInfo>> {
        let communities = self.state.communities.read();
        let community = communities.get(community_id)?;
        let gossip = community.gossip.as_ref()?;
        Some(
            gossip
                .peers
                .iter()
                .map(|(key, member)| PeerInfo {
                    pseudonym_key: key.clone(),
                    route_blob: member.route_blob.clone(),
                })
                .collect(),
        )
    }

    fn peer_reliability_scores(&self, community_id: &str) -> HashMap<String, f64> {
        let communities = self.state.communities.read();
        let Some(community) = communities.get(community_id) else {
            return HashMap::new();
        };
        rekindle_gossip::scores_from_counters(&community.peer_reliability)
    }

    fn online_member_status(&self, community_id: &str, peer_key: &str) -> Option<String> {
        let communities = self.state.communities.read();
        communities
            .get(community_id)?
            .gossip
            .as_ref()?
            .online_members
            .get(peer_key)
            .map(|m| m.status.clone())
    }

    fn enqueue_pending_mesh(&self, community_id: &str, signed: SignedEnvelope) {
        let mut communities = self.state.communities.write();
        let Some(community) = communities.get_mut(community_id) else {
            return;
        };
        let Some(ref mut gossip) = community.gossip else {
            return;
        };
        if gossip.pending_mesh_broadcasts.len() >= rekindle_gossip::MAX_PENDING_MESH {
            gossip.pending_mesh_broadcasts.pop_front();
        }
        gossip.pending_mesh_broadcasts.push_back(signed);
    }

    fn update_peer_route(
        &self,
        community_id: &str,
        peer_key: &str,
        status: &str,
        route_blob: Vec<u8>,
    ) {
        let mut communities = self.state.communities.write();
        if let Some(community) = communities.get_mut(community_id) {
            if let Some(ref mut gossip) = community.gossip {
                let now = rekindle_utils::timestamp_secs();
                let member = OnlineMember {
                    route_blob,
                    status: status.to_string(),
                    last_seen: now,
                };
                gossip
                    .online_members
                    .insert(peer_key.to_string(), member.clone());
                if gossip.peers.contains_key(peer_key) {
                    gossip.peers.insert(peer_key.to_string(), member);
                }
            }
        }
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
        let registry_key = {
            let communities = self.state.communities.read();
            let community = communities.get(community_id)?;
            community.member_registry_key.clone()?
        };

        let cid = community_id.to_string();
        let pk = peer_pseudonym.to_string();
        let subkey_index = crate::db_helpers::db_call(&self.pool, move |conn| {
            conn.query_row(
                "SELECT subkey_index FROM community_members WHERE community_id = ?1 AND pseudonym_key = ?2",
                rusqlite::params![cid, pk],
                |row| row.get::<_, u32>(0),
            )
            .ok()
            .ok_or(rusqlite::Error::QueryReturnedNoRows)
        })
        .await
        .ok()?;

        let rc = state_helpers::safe_routing_context(&self.state)?;
        let mgr = DHTManager::new(rc);
        match member_registry::read_member_presence_fresh(&mgr, &registry_key, subkey_index).await {
            Ok(Some(presence)) if presence.status != "offline" => {
                presence.route_blob.filter(|blob| !blob.is_empty())
            }
            _ => None,
        }
    }

    async fn send_app_message(&self, route_blob: &[u8], data: Vec<u8>) -> Result<(), String> {
        let rc = state_helpers::safe_routing_context(&self.state)
            .ok_or_else(|| "no routing context".to_string())?;
        let route_id = rc
            .api()
            .import_remote_private_route(route_blob.to_vec())
            .map_err(|e| e.to_string())?;
        rc.app_message(veilid_core::Target::RouteId(route_id), data)
            .await
            .map_err(|e| e.to_string())
    }
}

/// Build a one-shot adapter from the live AppState. The facade in
/// `services/community/gossip.rs` constructs this per call (cheap —
/// just clones two Arcs) and hands it to the crate's orchestrators.
pub fn build_adapter(state: &std::sync::Arc<crate::state::AppState>) -> Option<GossipAdapter> {
    let app_handle = state.app_handle.read().clone()?;
    let pool = app_handle.try_state::<crate::db::DbPool>()?.inner().clone();
    Some(GossipAdapter::new(std::sync::Arc::clone(state), pool))
}
