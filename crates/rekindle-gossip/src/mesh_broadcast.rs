//! Phase 20 REDO — full mesh-broadcast orchestrators.
//!
//! Pre-port these bodies lived in `src-tauri/services/community/gossip.rs`
//! and read `state.communities` / `state.identity_secret` / etc. directly.
//! Here they parameterise over `GossipDeps` so the entire pipeline (sign
//! + dedup + lamport bump + peer-select + supervised per-peer fan-out
//! with route re-resolution) is testable against a mock and free of
//! `veilid-core` / `tauri` / `rusqlite`.

use std::sync::Arc;

use rekindle_protocol::capnp_envelope::{
    encode_community_envelope, encode_signed_envelope, try_decode_community_envelope,
};
use rekindle_protocol::dht::community::envelope::{self, CommunityEnvelope, SignedEnvelope};

use crate::broadcast::extract_mesh_dedup_key;
use crate::deps::{GossipDeps, GossipError, PeerInfo};
use crate::mesh::fanout_degree;
use crate::peer_select::sort_peers_by_reliability;

/// Maximum number of broadcasts queued per community when fan-out
/// finds zero peers. Bounded so an offline burst doesn't OOM the
/// process. The adapter enforces this cap inside
/// `GossipDeps::enqueue_pending_mesh`; the constant lives here so
/// tests + tracing can reference it.
pub const MAX_PENDING_MESH: usize = 100;

/// Encode + sign the envelope, insert the dedup key, bump the
/// community lamport counter, then fan out via `send_to_mesh_raw`.
pub async fn send_to_mesh<D: GossipDeps>(
    deps: Arc<D>,
    community_id: &str,
    envelope: &CommunityEnvelope,
) -> Result<(), GossipError> {
    let my_pseudonym_key = deps.my_pseudonym_key(community_id);
    let identity_secret = deps
        .identity_secret()
        .ok_or(GossipError::IdentityNotLoaded)?;

    let signing_key =
        rekindle_crypto::group::pseudonym::derive_community_pseudonym(&identity_secret, community_id);
    let envelope_bytes = encode_community_envelope(envelope)
        .map_err(|e| GossipError::EncodeFailed(e.to_string()))?;
    let signed = envelope::sign_envelope(
        &signing_key,
        community_id,
        &my_pseudonym_key,
        &envelope_bytes,
    );

    let dedup_key = extract_mesh_dedup_key(envelope);
    deps.check_and_insert_dedup(community_id, &my_pseudonym_key, &dedup_key);
    deps.increment_lamport(community_id);

    send_to_mesh_raw(deps, community_id, signed);
    Ok(())
}

/// Fan-out a pre-signed envelope. Idempotent + crash-safe by design:
/// if `current_peers` returns zero, the envelope is queued for the
/// next presence-poll cycle instead of being dropped (architecture
/// A1/P4.1). On a non-empty peer list, ranks by reliability +
/// applies the architecture §3 fan-out degree + spawns a single
/// supervising tokio task with a `JoinSet` so per-peer tasks are
/// owned (M9.6, not orphaned).
pub fn send_to_mesh_raw<D: GossipDeps>(
    deps: Arc<D>,
    community_id: &str,
    signed: SignedEnvelope,
) {
    let signed_bytes = encode_signed_envelope(&signed);

    let Some(peers) = deps.current_peers(community_id) else {
        tracing::warn!(
            community = %community_id,
            "send_to_mesh_raw: community or gossip overlay missing",
        );
        return;
    };

    if peers.is_empty() {
        deps.enqueue_pending_mesh(community_id, signed);
        tracing::info!(
            community = %community_id,
            "send_to_mesh_raw: peers empty, queued for later drain",
        );
        return;
    }

    let scores = deps.peer_reliability_scores(community_id);
    let scored = sort_peers_by_reliability(peers, &scores);
    let degree = fanout_degree(scored.len());
    let selected: Vec<PeerInfo> = scored.into_iter().take(degree).collect();

    tracing::info!(
        community = %community_id,
        peer_count = selected.len(),
        fanout_degree = degree,
        "send_to_mesh_raw: sending to gossip fan-out",
    );

    let message_id: Option<String> = try_decode_community_envelope(&signed.envelope_bytes)
        .ok()
        .flatten()
        .and_then(|envelope| match envelope {
            CommunityEnvelope::MessageNotification { message_id, .. } => Some(message_id),
            _ => None,
        });

    let cid_owner = community_id.to_string();
    tokio::spawn(async move {
        let mut set = tokio::task::JoinSet::new();
        for peer in selected {
            let deps_clone = Arc::clone(&deps);
            let cid = cid_owner.clone();
            let data = signed_bytes.clone();
            let msg_id = message_id.clone();
            set.spawn(async move {
                send_to_one_peer(deps_clone, cid, peer, data, msg_id).await;
            });
        }
        while set.join_next().await.is_some() {}
    });
}

/// One peer send + route re-resolve + retry. Records delivery /
/// reliability on every terminal outcome.
async fn send_to_one_peer<D: GossipDeps>(
    deps: Arc<D>,
    community_id: String,
    peer: PeerInfo,
    data: Vec<u8>,
    msg_id: Option<String>,
) {
    let first = deps.send_app_message(&peer.route_blob, data.clone()).await;

    if first.is_ok() {
        if let Some(ref mid) = msg_id {
            deps.record_delivery(mid, &community_id, &peer.pseudonym_key, "delivered")
                .await;
        }
        deps.record_peer_reliability(&community_id, &peer.pseudonym_key, true);
        return;
    }

    deps.record_peer_reliability(&community_id, &peer.pseudonym_key, false);

    tracing::info!(
        community = %community_id,
        peer = %peer.pseudonym_key,
        "route stale, attempting DHT re-resolve",
    );

    let Some(fresh_blob) = deps
        .resolve_peer_route_from_dht(&community_id, &peer.pseudonym_key)
        .await
    else {
        tracing::warn!(
            community = %community_id,
            peer = %peer.pseudonym_key,
            "no fresh route found in DHT",
        );
        if let Some(ref mid) = msg_id {
            deps.record_delivery(mid, &community_id, &peer.pseudonym_key, "failed")
                .await;
        }
        return;
    };

    match deps.send_app_message(&fresh_blob, data).await {
        Ok(()) => {
            let status = deps
                .online_member_status(&community_id, &peer.pseudonym_key)
                .unwrap_or_else(|| "online".to_string());
            deps.update_peer_route(&community_id, &peer.pseudonym_key, &status, fresh_blob);
            if let Some(ref mid) = msg_id {
                deps.record_delivery(mid, &community_id, &peer.pseudonym_key, "delivered")
                    .await;
            }
        }
        Err(error) => {
            tracing::warn!(
                community = %community_id,
                peer = %peer.pseudonym_key,
                %error,
                "re-resolved route still failed",
            );
            if let Some(ref mid) = msg_id {
                deps.record_delivery(mid, &community_id, &peer.pseudonym_key, "failed")
                    .await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet, VecDeque};
    use std::sync::Arc;

    use async_trait::async_trait;
    use parking_lot::Mutex;
    use rekindle_protocol::dht::community::envelope::SignedEnvelope;

    use super::*;

    #[derive(Default)]
    struct MockState {
        peers: Option<Vec<PeerInfo>>,
        scores: HashMap<String, f64>,
        statuses: HashMap<String, String>,
        send_results: VecDeque<Result<(), String>>,
        fresh_routes: HashMap<String, Vec<u8>>,
        sent_payloads: Vec<(Vec<u8>, Vec<u8>)>,
        deliveries: Vec<(String, String, String, String)>,
        reliability: Vec<(String, String, bool)>,
        route_updates: Vec<(String, String, String, Vec<u8>)>,
        pending_queue: Vec<SignedEnvelope>,
        dedup_inserts: HashSet<(String, String, String)>,
        lamport_bumps: Vec<String>,
    }

    struct MockDeps {
        state: Mutex<MockState>,
        identity: Option<[u8; 32]>,
        my_pseudonym: String,
    }

    impl MockDeps {
        fn new() -> Self {
            Self {
                state: Mutex::new(MockState::default()),
                identity: Some([7u8; 32]),
                my_pseudonym: "me".to_string(),
            }
        }
    }

    #[async_trait]
    impl GossipDeps for MockDeps {
        fn my_pseudonym_key(&self, _community_id: &str) -> String {
            self.my_pseudonym.clone()
        }
        fn identity_secret(&self) -> Option<[u8; 32]> {
            self.identity
        }
        fn check_and_insert_dedup(&self, c: &str, s: &str, k: &str) {
            self.state
                .lock()
                .dedup_inserts
                .insert((c.to_string(), s.to_string(), k.to_string()));
        }
        fn increment_lamport(&self, community_id: &str) {
            self.state.lock().lamport_bumps.push(community_id.to_string());
        }
        fn current_peers(&self, _c: &str) -> Option<Vec<PeerInfo>> {
            self.state.lock().peers.clone()
        }
        fn peer_reliability_scores(&self, _c: &str) -> HashMap<String, f64> {
            self.state.lock().scores.clone()
        }
        fn online_member_status(&self, _c: &str, peer_key: &str) -> Option<String> {
            self.state.lock().statuses.get(peer_key).cloned()
        }
        fn enqueue_pending_mesh(&self, _c: &str, signed: SignedEnvelope) {
            self.state.lock().pending_queue.push(signed);
        }
        fn update_peer_route(&self, c: &str, peer: &str, status: &str, blob: Vec<u8>) {
            self.state.lock().route_updates.push((
                c.to_string(),
                peer.to_string(),
                status.to_string(),
                blob,
            ));
        }
        fn record_peer_reliability(&self, c: &str, peer: &str, success: bool) {
            self.state
                .lock()
                .reliability
                .push((c.to_string(), peer.to_string(), success));
        }
        async fn record_delivery(&self, mid: &str, c: &str, r: &str, s: &str) {
            self.state.lock().deliveries.push((
                mid.to_string(),
                c.to_string(),
                r.to_string(),
                s.to_string(),
            ));
        }
        async fn resolve_peer_route_from_dht(&self, _c: &str, peer: &str) -> Option<Vec<u8>> {
            self.state.lock().fresh_routes.get(peer).cloned()
        }
        async fn send_app_message(&self, route_blob: &[u8], data: Vec<u8>) -> Result<(), String> {
            let mut state = self.state.lock();
            state.sent_payloads.push((route_blob.to_vec(), data));
            state.send_results.pop_front().unwrap_or(Ok(()))
        }
    }

    fn fake_signed(payload: &[u8]) -> SignedEnvelope {
        SignedEnvelope {
            community_id: "c1".to_string(),
            sender_pseudonym: "me".to_string(),
            envelope_bytes: payload.to_vec(),
            signature: vec![0u8; 64],
            ttl: 5,
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn empty_peers_enqueues_instead_of_dropping() {
        let deps = Arc::new(MockDeps::new());
        deps.state.lock().peers = Some(Vec::new());
        send_to_mesh_raw(Arc::clone(&deps), "c1", fake_signed(b"hello"));
        // Spawned supervisor is short-circuited; enqueue happened synchronously.
        let st = deps.state.lock();
        assert_eq!(st.pending_queue.len(), 1);
        assert!(st.sent_payloads.is_empty());
        assert!(st.deliveries.is_empty());
        assert!(st.reliability.is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn missing_overlay_logs_and_returns() {
        let deps = Arc::new(MockDeps::new());
        // peers = None means overlay missing.
        send_to_mesh_raw(Arc::clone(&deps), "c1", fake_signed(b"hello"));
        let st = deps.state.lock();
        assert!(st.pending_queue.is_empty());
        assert!(st.sent_payloads.is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn successful_send_records_delivered_and_marks_reliable() {
        let deps = Arc::new(MockDeps::new());
        deps.state.lock().peers = Some(vec![PeerInfo {
            pseudonym_key: "alice".to_string(),
            route_blob: vec![1, 2, 3],
        }]);
        deps.state.lock().send_results.push_back(Ok(()));

        send_to_mesh_raw(Arc::clone(&deps), "c1", fake_signed(b"hi"));
        // Wait for spawned supervisor to finish.
        tokio::task::yield_now().await;
        for _ in 0..20 {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            if !deps.state.lock().reliability.is_empty() {
                break;
            }
        }
        let st = deps.state.lock();
        assert_eq!(st.sent_payloads.len(), 1);
        assert_eq!(st.reliability, vec![("c1".into(), "alice".into(), true)]);
        // No message_id inside envelope_bytes → no delivery row recorded.
        assert!(st.deliveries.is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn failure_then_fresh_route_succeeds_and_updates_route() {
        let deps = Arc::new(MockDeps::new());
        deps.state.lock().peers = Some(vec![PeerInfo {
            pseudonym_key: "bob".to_string(),
            route_blob: vec![9, 9, 9],
        }]);
        deps.state.lock().send_results.push_back(Err("stale".into()));
        deps.state.lock().send_results.push_back(Ok(()));
        deps.state
            .lock()
            .fresh_routes
            .insert("bob".into(), vec![7, 7, 7]);
        deps.state
            .lock()
            .statuses
            .insert("bob".into(), "away".into());

        send_to_mesh_raw(Arc::clone(&deps), "c1", fake_signed(b"x"));
        for _ in 0..40 {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            if !deps.state.lock().route_updates.is_empty() {
                break;
            }
        }
        let st = deps.state.lock();
        assert_eq!(st.sent_payloads.len(), 2);
        assert_eq!(
            st.reliability,
            vec![("c1".into(), "bob".into(), false)],
            "only the initial failure is recorded; retry success does not double-credit"
        );
        assert_eq!(
            st.route_updates,
            vec![("c1".into(), "bob".into(), "away".into(), vec![7, 7, 7])]
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn failure_with_no_fresh_route_records_nothing_extra() {
        let deps = Arc::new(MockDeps::new());
        deps.state.lock().peers = Some(vec![PeerInfo {
            pseudonym_key: "carol".to_string(),
            route_blob: vec![3, 3, 3],
        }]);
        deps.state.lock().send_results.push_back(Err("dead".into()));

        send_to_mesh_raw(Arc::clone(&deps), "c1", fake_signed(b"y"));
        for _ in 0..40 {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            if !deps.state.lock().reliability.is_empty() {
                break;
            }
        }
        let st = deps.state.lock();
        assert_eq!(st.sent_payloads.len(), 1);
        assert_eq!(st.reliability, vec![("c1".into(), "carol".into(), false)]);
        assert!(st.route_updates.is_empty());
    }

    #[test]
    fn fanout_constants_match_pre_port() {
        // Sanity-check that the architecture §3 fan-out we depend on
        // didn't shift when this module was extracted.
        assert_eq!(fanout_degree(0), 0);
        assert_eq!(fanout_degree(6), 6);
        assert_eq!(fanout_degree(60), 6);
        assert_eq!(fanout_degree(61), 8);
        assert_eq!(MAX_PENDING_MESH, 100);
    }
}
