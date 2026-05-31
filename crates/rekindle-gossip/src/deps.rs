//! Phase 20 REDO — `GossipDeps` composite trait and supporting DTOs.
//!
//! The `mesh_broadcast` orchestrators are parameterised over this
//! trait so the crate stays free of `veilid-core`, `tauri`, and
//! `rusqlite` while still performing the full gossip pipeline
//! (sign + dedup + lamport-bump + peer-select + supervised fan-out
//! with route re-resolution on failure).

use std::collections::HashMap;

use async_trait::async_trait;
use rekindle_protocol::dht::community::envelope::SignedEnvelope;

/// One candidate peer in the gossip mesh.
#[derive(Debug, Clone)]
pub struct PeerInfo {
    pub pseudonym_key: String,
    pub route_blob: Vec<u8>,
}

/// Errors surfaced by the orchestrator's public entry points.
#[derive(Debug, thiserror::Error)]
pub enum GossipError {
    #[error("identity not unlocked")]
    IdentityNotLoaded,
    #[error("community not found: {0}")]
    CommunityNotFound(String),
    #[error("encode envelope: {0}")]
    EncodeFailed(String),
}

/// Bag of operations the crate needs from its host. Implemented
/// in src-tauri by `GossipAdapter` against the live `AppState` +
/// `AppHandle` + `DbPool`.
#[async_trait]
pub trait GossipDeps: Send + Sync + 'static {
    // === Identity / community state ===
    /// Current member's pseudonym key for the given community.
    /// Empty string if absent (matches the pre-port `unwrap_or_default`).
    fn my_pseudonym_key(&self, community_id: &str) -> String;

    /// Locked identity secret bytes, or `None` if vault is locked.
    fn identity_secret(&self) -> Option<[u8; 32]>;

    // === Dedup / Lamport (interior mutability) ===
    fn check_and_insert_dedup(&self, community_id: &str, sender: &str, dedup_key: &str);
    fn increment_lamport(&self, community_id: &str);

    // === Peer overlay reads ===
    /// Snapshot of the current gossip peers for the community, or
    /// `None` if the community / gossip overlay is missing.
    fn current_peers(&self, community_id: &str) -> Option<Vec<PeerInfo>>;

    /// Pseudonym → reliability score in `[0.0, 1.0]`. Peers absent
    /// from the map are treated as neutral (0.5) by `peer_select`.
    fn peer_reliability_scores(&self, community_id: &str) -> HashMap<String, f64>;

    /// Last-known status string ("online" / "away" / etc.) for a peer
    /// in the community, or `None` if unknown.
    fn online_member_status(&self, community_id: &str, peer_key: &str) -> Option<String>;

    // === Peer overlay mutations ===
    /// Push a signed envelope onto the per-community pending-mesh
    /// queue when fan-out finds zero peers.
    fn enqueue_pending_mesh(&self, community_id: &str, signed: SignedEnvelope);

    /// Patch a peer's route_blob after re-resolving from DHT. The
    /// implementation also updates the `online_members` mirror so the
    /// next presence-poll cycle preserves the fresh route.
    fn update_peer_route(
        &self,
        community_id: &str,
        peer_key: &str,
        status: &str,
        route_blob: Vec<u8>,
    );

    /// Bump a peer's reliability counters and mark dirty for the
    /// next periodic SQLite flush.
    fn record_peer_reliability(&self, community_id: &str, peer_key: &str, success: bool);

    // === Persistence (async) ===
    /// Upsert a `message_delivery` row tagged with the attempt outcome.
    async fn record_delivery(
        &self,
        message_id: &str,
        community_id: &str,
        recipient: &str,
        status: &str,
    );

    /// Look up the peer's fresh route_blob via the community member
    /// registry. Returns `None` if no valid blob is published.
    async fn resolve_peer_route_from_dht(
        &self,
        community_id: &str,
        peer_pseudonym: &str,
    ) -> Option<Vec<u8>>;

    // === Transport (async) ===
    /// Import the remote private route and send the payload via
    /// `app_message`. Returns `Err` if the send fails for any reason
    /// (route stale, network drop, peer offline).
    async fn send_app_message(&self, route_blob: &[u8], data: Vec<u8>) -> Result<(), String>;
}
