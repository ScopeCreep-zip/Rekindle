use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use rekindle_codec::community::envelope::SignedEnvelope;

use super::*;
use crate::channel_send::send_to_channel_peers;

#[derive(Default)]
struct MockState {
    peers: Option<Vec<PeerInfo>>,
    scores: HashMap<String, f64>,
    statuses: HashMap<String, String>,
    send_results: VecDeque<Result<(), String>>,
    fresh_routes: HashMap<String, Vec<u8>>,
    sent_payloads: Vec<(Vec<u8>, Vec<u8>)>,
    reliability: Vec<(String, String, bool)>,
    route_updates: Vec<(String, String, String, Vec<u8>)>,
    pending_queue: Vec<SignedEnvelope>,
    dedup_inserts: HashSet<(String, String, String)>,
}

struct MockDeps {
    state: Mutex<MockState>,
    identity: Option<[u8; 32]>,
    my_pseudonym: String,
    resolve_gate: crate::resolve_gate::ResolveGate,
    scope: Arc<rekindle_lifecycle::SessionScope>,
}

impl MockDeps {
    fn new() -> Self {
        Self {
            state: Mutex::new(MockState::default()),
            identity: Some([7u8; 32]),
            my_pseudonym: "me".to_string(),
            resolve_gate: crate::resolve_gate::ResolveGate::new(),
            scope: rekindle_lifecycle::SessionScope::new("test", Arc::new(|_| {})),
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
    fn scope(&self) -> Arc<rekindle_lifecycle::SessionScope> {
        Arc::clone(&self.scope)
    }
    fn rng(&self) -> Arc<dyn rekindle_utils::RandomSource> {
        Arc::new(rekindle_utils::OsRandom)
    }
    fn check_and_insert_dedup(&self, c: &str, s: &str, k: &str) {
        self.state
            .lock()
            .dedup_inserts
            .insert((c.to_string(), s.to_string(), k.to_string()));
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
    async fn resolve_peer_route_from_dht(&self, _c: &str, peer: &str) -> Option<Vec<u8>> {
        self.state.lock().fresh_routes.get(peer).cloned()
    }
    fn resolve_gate(&self) -> &crate::resolve_gate::ResolveGate {
        &self.resolve_gate
    }
    async fn send_app_message(&self, route_blob: &[u8], data: Vec<u8>) -> Result<(), String> {
        let mut state = self.state.lock();
        state.sent_payloads.push((route_blob.to_vec(), data));
        state.send_results.pop_front().unwrap_or(Ok(()))
    }
}

fn fake_signed(payload: &[u8]) -> SignedEnvelope {
    SignedEnvelope {
        message_id: [7u8; 16],
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
    send_to_mesh_raw(Arc::clone(&deps), "c1", fake_signed(b"hello")).unwrap();
    // Spawned supervisor is short-circuited; enqueue happened synchronously.
    let st = deps.state.lock();
    assert_eq!(st.pending_queue.len(), 1);
    assert!(st.sent_payloads.is_empty());
    assert!(st.reliability.is_empty());
}

/// A send before the overlay is built (just joined, no presence poll
/// yet) is held for the drain, not dropped.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_overlay_enqueues_for_the_drain() {
    let deps = Arc::new(MockDeps::new());
    // peers = None means overlay missing.
    send_to_mesh_raw(Arc::clone(&deps), "c1", fake_signed(b"hello")).unwrap();
    let st = deps.state.lock();
    assert_eq!(st.pending_queue.len(), 1);
    assert!(st.sent_payloads.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn successful_send_marks_reliable() {
    let deps = Arc::new(MockDeps::new());
    deps.state.lock().peers = Some(vec![PeerInfo {
        pseudonym_key: "alice".to_string(),
        route_blob: vec![1, 2, 3],
    }]);
    deps.state.lock().send_results.push_back(Ok(()));

    send_to_mesh_raw(Arc::clone(&deps), "c1", fake_signed(b"hi")).unwrap();
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
}

/// The awaited send reaches its peers with the session's scope closed
/// (the quit at a teardown, Linux W5b): the spawned fan-out is dropped.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_awaited_send_reaches_peers_after_the_scope_closed() {
    let deps = Arc::new(MockDeps::new());
    deps.scope
        .shutdown(std::time::Duration::from_secs(1))
        .await
        .expect("stops");
    deps.state.lock().peers = Some(vec![PeerInfo {
        pseudonym_key: "alice".to_string(),
        route_blob: vec![1, 2, 3],
    }]);
    let envelope =
        CommunityEnvelope::control(envelope::ControlPayload::HeadAdvanced { hint: vec![1] });
    let sent = send_to_mesh_awaited(Arc::clone(&deps), "c1", &envelope)
        .await
        .expect("signed");
    assert_eq!(sent, 1);
    assert_eq!(
        deps.state.lock().sent_payloads.len(),
        1,
        "sent before returning"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failure_then_fresh_route_succeeds_and_updates_route() {
    let deps = Arc::new(MockDeps::new());
    deps.state.lock().peers = Some(vec![PeerInfo {
        pseudonym_key: "bob".to_string(),
        route_blob: vec![9, 9, 9],
    }]);
    deps.state
        .lock()
        .send_results
        .push_back(Err("stale".into()));
    deps.state.lock().send_results.push_back(Ok(()));
    deps.state
        .lock()
        .fresh_routes
        .insert("bob".into(), vec![7, 7, 7]);
    deps.state
        .lock()
        .statuses
        .insert("bob".into(), "away".into());

    send_to_mesh_raw(Arc::clone(&deps), "c1", fake_signed(b"x")).unwrap();
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

    send_to_mesh_raw(Arc::clone(&deps), "c1", fake_signed(b"y")).unwrap();
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn channel_peers_send_reaches_every_peer_with_ttl_zero() {
    use rekindle_codec::capnp_envelope::decode_signed_envelope;
    use rekindle_codec::ingress::{classify, Ingress};

    let deps = Arc::new(MockDeps::new());
    let peers: Vec<PeerInfo> = (0..3u8)
        .map(|i| PeerInfo {
            pseudonym_key: format!("peer{i}"),
            route_blob: vec![i; 3],
        })
        .collect();
    let envelope = CommunityEnvelope::TypingIndicator {
        channel_tag: [1; 16],
        ttl_ms: 6_000,
        pseudonym_key: "me".to_string(),
    };

    send_to_channel_peers(Arc::clone(&deps), "c1", &envelope, peers)
        .await
        .expect("directed send happy path");
    for _ in 0..40 {
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        if deps.state.lock().sent_payloads.len() == 3 {
            break;
        }
    }
    let st = deps.state.lock();
    assert_eq!(
        st.sent_payloads.len(),
        3,
        "every channel peer gets the envelope — no fan-out degree cap"
    );
    for (_route, data) in &st.sent_payloads {
        let Ingress::Gossip(bytes) = classify(data) else {
            panic!("a channel send is a 'G' gossip datagram");
        };
        let signed = decode_signed_envelope(bytes).expect("wire-decodable");
        assert_eq!(signed.ttl, 0, "directed media must not be relayable");
        assert_eq!(signed.community_id, "c1");
    }
    assert!(st.pending_queue.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn channel_peers_send_with_empty_roster_drops_instead_of_queueing() {
    let deps = Arc::new(MockDeps::new());
    let envelope = CommunityEnvelope::TypingIndicator {
        channel_tag: [1; 16],
        ttl_ms: 6_000,
        pseudonym_key: "me".to_string(),
    };
    send_to_channel_peers(Arc::clone(&deps), "c1", &envelope, Vec::new())
        .await
        .expect("empty roster is not an error");
    let st = deps.state.lock();
    assert!(
        st.pending_queue.is_empty(),
        "live media is dropped, never deferred to the pending-mesh queue"
    );
    assert!(st.sent_payloads.is_empty());
}

/// Two sends of identical content are two messages: each signs a fresh
/// id and records it, so neither our cache nor a receiver's suppresses
/// the second (the roster reconcile's re-sent join).
#[test]
fn each_send_of_the_same_content_gets_a_fresh_message_id() {
    let deps = Arc::new(MockDeps::new());
    let envelope = CommunityEnvelope::TypingIndicator {
        channel_tag: [1; 16],
        ttl_ms: 6_000,
        pseudonym_key: "me".to_string(),
    };
    let first = sign_and_record(&deps, "c1", &envelope).expect("signs");
    let second = sign_and_record(&deps, "c1", &envelope).expect("signs");
    assert_eq!(first.envelope_bytes, second.envelope_bytes);
    assert_ne!(first.message_id, second.message_id);
    assert_eq!(deps.state.lock().dedup_inserts.len(), 2);
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
