//! Phase 14.j regression tests — handler-level integration coverage.
//!
//! These tests lock in the audit-fixed behaviors caught during the
//! Phase 13-style audit loop:
//!   - W14.1: `handle_accept_received` must invoke
//!     `deps.pre_stage_voice_channel()` BEFORE the
//!     `deps.start_voice_session(...)` await. Without this, voice
//!     packets arriving during the accept handler drop at dispatch.
//!   - W14.2: `handle_accept_received` must emit `CallConnected`
//!     carrying the call `kind` so the adapter maps
//!     `expected_local_camera = matches!(kind, Video)`. Without this,
//!     video calls never start local WebCodecs camera.
//!   - Group accept: `handle_group_accept_received` must transition
//!     `GroupCallState.status` to `Active` on the first accept (not
//!     just emit `GroupCallConnected`).

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use rekindle_protocol::messaging::envelope::MessagePayload;
use x25519_dalek::{PublicKey, StaticSecret};

use crate::error::CallError;
use crate::group_state::{GroupCallState, GroupCallStatus};
use crate::signaling::deps::CallSignalingDeps;
use crate::signaling::event::CallSignalEvent;
use crate::signaling::registry::{CallRegistry, GroupCallRegistry, GroupCallSnapshot};
use crate::state::{CallKind, CallState, CallStatus};

#[derive(Debug, Clone, PartialEq, Eq)]
enum MockEvent {
    PreStage,
    StartVoiceSession {
        call_id: String,
        kind: CallKind,
    },
    SpawnIncomingTimeout {
        call_id: String,
        peer_pubkey: String,
        kind: CallKind,
        expires_at_ms: u64,
    },
    Emit(String), // discriminant only — full payload in EmittedEvents
}

struct MockState {
    /// Ordered log of trait method invocations (used to verify W14.1
    /// ordering: PreStage MUST precede StartVoiceSession).
    call_log: Vec<MockEvent>,
    emitted: Vec<CallSignalEvent>,
    start_voice_session_result: Result<(), CallError>,
}

impl Default for MockState {
    fn default() -> Self {
        Self {
            call_log: Vec::new(),
            emitted: Vec::new(),
            start_voice_session_result: Ok(()),
        }
    }
}

struct MockRegistry {
    inner: Mutex<HashMap<String, CallState>>,
}
impl CallRegistry for MockRegistry {
    fn insert(&self, call: CallState) {
        self.inner.lock().insert(call.call_id.clone(), call);
    }
    fn get(&self, call_id: &str) -> Option<CallState> {
        self.inner.lock().get(call_id).cloned()
    }
    fn remove(&self, call_id: &str) -> Option<CallState> {
        self.inner.lock().remove(call_id)
    }
    fn contains(&self, call_id: &str) -> bool {
        self.inner.lock().contains_key(call_id)
    }
    fn outgoing_to_peer(&self, peer: &str) -> Option<CallState> {
        self.inner
            .lock()
            .values()
            .find(|c| c.peer_pubkey == peer && matches!(c.status, CallStatus::Outgoing))
            .cloned()
    }
    fn list_all(&self) -> Vec<CallState> {
        self.inner.lock().values().cloned().collect()
    }
}

struct MockGroupRegistry {
    inner: Mutex<HashMap<String, GroupCallState>>,
}
impl GroupCallRegistry for MockGroupRegistry {
    fn insert(&self, call: GroupCallState) {
        self.inner.lock().insert(call.call_id.clone(), call);
    }
    fn remove(&self, call_id: &str) -> Option<GroupCallState> {
        self.inner.lock().remove(call_id)
    }
    fn contains(&self, call_id: &str) -> bool {
        self.inner.lock().contains_key(call_id)
    }
    fn add_accept(&self, call_id: &str, peer: &str) -> bool {
        let mut g = self.inner.lock();
        let Some(call) = g.get_mut(call_id) else {
            return false;
        };
        let was_empty = call.accepted.is_empty();
        call.accepted.insert(peer.to_string());
        was_empty
    }
    fn set_status(&self, call_id: &str, status: GroupCallStatus) {
        if let Some(call) = self.inner.lock().get_mut(call_id) {
            call.status = status;
        }
    }
    fn snapshot(&self, call_id: &str) -> Option<GroupCallSnapshot> {
        let g = self.inner.lock();
        let c = g.get(call_id)?;
        Some(GroupCallSnapshot {
            call_id: c.call_id.clone(),
            initiator_pubkey: c.initiator_pubkey.clone(),
            kind: c.kind,
            participants: c.participants.clone(),
            accepted_count: c.accepted.len(),
            status: c.status,
        })
    }
}

struct MockDeps {
    state: Mutex<MockState>,
    registry: Arc<MockRegistry>,
    group_registry: Arc<MockGroupRegistry>,
    owner_key: String,
    identity_secret: [u8; 32],
}

impl MockDeps {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(MockState {
                start_voice_session_result: Ok(()),
                ..Default::default()
            }),
            registry: Arc::new(MockRegistry {
                inner: Mutex::new(HashMap::new()),
            }),
            group_registry: Arc::new(MockGroupRegistry {
                inner: Mutex::new(HashMap::new()),
            }),
            owner_key: "aa".repeat(32),
            identity_secret: [0xCC; 32],
        })
    }

    fn call_log(&self) -> Vec<MockEvent> {
        self.state.lock().call_log.clone()
    }
    fn emitted_events(&self) -> Vec<CallSignalEvent> {
        self.state.lock().emitted.clone()
    }
}

#[async_trait]
impl CallSignalingDeps for MockDeps {
    fn owner_key(&self) -> Result<String, CallError> {
        Ok(self.owner_key.clone())
    }
    fn identity_secret(&self) -> Result<[u8; 32], CallError> {
        Ok(self.identity_secret)
    }
    fn registry(&self) -> Arc<dyn CallRegistry> {
        Arc::clone(&self.registry) as Arc<dyn CallRegistry>
    }
    fn group_registry(&self) -> Arc<dyn GroupCallRegistry> {
        Arc::clone(&self.group_registry) as Arc<dyn GroupCallRegistry>
    }
    fn is_peer_temp_muted(&self, _peer: &str) -> bool {
        false
    }
    fn friend_display_name(&self, _peer: &str) -> String {
        "Mock Friend".into()
    }
    async fn send_to_peer(&self, _peer: &str, _payload: MessagePayload) -> Result<(), CallError> {
        Ok(())
    }
    async fn start_voice_session(
        &self,
        call_id: &str,
        _peer: &str,
        _call_key: [u8; 32],
        kind: CallKind,
    ) -> Result<(), CallError> {
        let mut s = self.state.lock();
        s.call_log.push(MockEvent::StartVoiceSession {
            call_id: call_id.to_string(),
            kind,
        });
        std::mem::replace(&mut s.start_voice_session_result, Ok(()))
    }
    async fn shutdown_voice_session(&self) {}
    fn voice_active(&self) -> bool {
        false
    }
    fn pre_stage_voice_channel(&self) {
        self.state.lock().call_log.push(MockEvent::PreStage);
    }
    fn persist_missed_call(&self, _: &str, _: &str, _: CallKind, _: u64) {}
    fn surface_window_for_call(&self, _: &str) {}
    fn emit_event(&self, event: CallSignalEvent) {
        let mut s = self.state.lock();
        let label = match &event {
            CallSignalEvent::IncomingCall { .. } => "IncomingCall",
            CallSignalEvent::CallRinging { .. } => "CallRinging",
            CallSignalEvent::CallConnected { .. } => "CallConnected",
            CallSignalEvent::CallDeclined { .. } => "CallDeclined",
            CallSignalEvent::CallEnded { .. } => "CallEnded",
            CallSignalEvent::CallTimedOut { .. } => "CallTimedOut",
            CallSignalEvent::CallMissed { .. } => "CallMissed",
            CallSignalEvent::ConversationFocusRequested { .. } => "ConversationFocusRequested",
            CallSignalEvent::CallStarted { .. } => "CallStarted",
            CallSignalEvent::IncomingGroupCall { .. } => "IncomingGroupCall",
            CallSignalEvent::GroupCallConnected { .. } => "GroupCallConnected",
            CallSignalEvent::GroupCallParticipantJoined { .. } => "GroupCallParticipantJoined",
            CallSignalEvent::GroupCallParticipantLeft { .. } => "GroupCallParticipantLeft",
            CallSignalEvent::GroupCallEnded { .. } => "GroupCallEnded",
        };
        s.call_log.push(MockEvent::Emit(label.to_string()));
        s.emitted.push(event);
    }
    fn register_background_handle(&self, _handle: tokio::task::JoinHandle<()>) {}
    fn spawn_incoming_call_timeout(
        &self,
        call_id: String,
        peer_pubkey: String,
        kind: CallKind,
        expires_at_ms: u64,
    ) {
        // Mock records the call but does NOT spawn anything (would
        // need its own runtime). Tests verify the spawn was invoked
        // with the right args.
        self.state
            .lock()
            .call_log
            .push(MockEvent::SpawnIncomingTimeout {
                call_id,
                peer_pubkey,
                kind,
                expires_at_ms,
            });
    }
    fn spawn_dialing_call_timeout(
        &self,
        _call_id: String,
        _peer_pubkey: String,
        _kind: CallKind,
        _expires_at_ms: u64,
    ) {
        // Mock no-op; existing regression tests don't exercise the
        // outbound (caller-side) flow yet.
    }
}

/// Set up an Outgoing CallState in the registry — what `start_dm_call`
/// would have done before this peer's `CallAccept` arrives.
fn seed_outgoing_call(deps: &MockDeps, call_id: &str, peer_hex: &str, kind: CallKind) {
    let my_secret = StaticSecret::random_from_rng(rand::rngs::OsRng);
    deps.registry.insert(CallState {
        call_id: call_id.to_string(),
        peer_pubkey: peer_hex.to_string(),
        kind,
        status: CallStatus::Outgoing,
        expires_at_ms: 9_999_999_999_999,
        my_x25519_secret: Some(my_secret),
        peer_x25519_pub: None,
        call_key: None,
    });
}

fn peer_x25519_pub_bytes() -> [u8; 32] {
    let secret = StaticSecret::random_from_rng(rand::rngs::OsRng);
    PublicKey::from(&secret).to_bytes()
}

// ─── W14.1 regression: pre-stage MUST precede start_voice_session ─────

#[tokio::test]
async fn w14_1_pre_stage_runs_before_start_voice_session() {
    let deps = MockDeps::new();
    seed_outgoing_call(&deps, "call-1", &"bb".repeat(32), CallKind::Audio);

    let acceptor_pub = peer_x25519_pub_bytes();
    crate::signaling::handlers::handle_accept_received(
        deps.as_ref(),
        &"bb".repeat(32),
        "call-1",
        &acceptor_pub,
    )
    .await;

    let log = deps.call_log();
    let pre_idx = log
        .iter()
        .position(|e| matches!(e, MockEvent::PreStage))
        .expect("pre_stage_voice_channel must be called");
    let start_idx = log
        .iter()
        .position(|e| matches!(e, MockEvent::StartVoiceSession { .. }))
        .expect("start_voice_session must be called");
    assert!(
        pre_idx < start_idx,
        "W14.1 violated: pre_stage_voice_channel (idx {pre_idx}) must precede \
         start_voice_session (idx {start_idx}). Order: {log:?}"
    );
}

// ─── W14.2 regression: CallConnected carries the kind ────────────────

#[tokio::test]
async fn w14_2_call_connected_carries_kind_audio() {
    let deps = MockDeps::new();
    seed_outgoing_call(&deps, "call-a", &"bb".repeat(32), CallKind::Audio);
    let acceptor_pub = peer_x25519_pub_bytes();

    crate::signaling::handlers::handle_accept_received(
        deps.as_ref(),
        &"bb".repeat(32),
        "call-a",
        &acceptor_pub,
    )
    .await;

    let kind = deps
        .emitted_events()
        .into_iter()
        .find_map(|e| match e {
            CallSignalEvent::CallConnected { kind, .. } => Some(kind),
            _ => None,
        })
        .expect("CallConnected must be emitted");
    assert_eq!(kind, CallKind::Audio);
}

#[tokio::test]
async fn w14_2_call_connected_carries_kind_video() {
    let deps = MockDeps::new();
    seed_outgoing_call(&deps, "call-v", &"bb".repeat(32), CallKind::Video);
    let acceptor_pub = peer_x25519_pub_bytes();

    crate::signaling::handlers::handle_accept_received(
        deps.as_ref(),
        &"bb".repeat(32),
        "call-v",
        &acceptor_pub,
    )
    .await;

    let kind = deps
        .emitted_events()
        .into_iter()
        .find_map(|e| match e {
            CallSignalEvent::CallConnected { kind, .. } => Some(kind),
            _ => None,
        })
        .expect("CallConnected must be emitted");
    assert_eq!(kind, CallKind::Video);
}

// ─── Group accept regression: status MUST become Active ──────────────

#[tokio::test]
async fn group_accept_first_acceptor_transitions_to_active() {
    let deps = MockDeps::new();
    let initiator = "aa".repeat(32);
    let participant1 = "bb".repeat(32);
    let participant2 = "cc".repeat(32);

    // Seed outgoing group call — initiator dialed, waiting for accepts.
    deps.group_registry.insert(GroupCallState {
        call_id: "group-1".into(),
        initiator_pubkey: initiator.clone(),
        kind: 0,
        participants: vec![initiator, participant1.clone(), participant2.clone()],
        accepted: std::collections::HashSet::new(),
        our_x25519_secret: Some(StaticSecret::random_from_rng(rand::rngs::OsRng)),
        call_key: Some([0xAA; 32]),
        status: GroupCallStatus::Outgoing,
    });

    crate::signaling::group_handlers::handle_group_accept_received(
        deps.as_ref(),
        &participant1,
        "group-1",
        &participant1,
    );

    let snapshot = deps
        .group_registry
        .snapshot("group-1")
        .expect("group call still in registry");
    assert_eq!(
        snapshot.status,
        GroupCallStatus::Active,
        "First-accept must transition status Outgoing → Active. Got: {:?}",
        snapshot.status
    );
    assert_eq!(snapshot.accepted_count, 1, "accept must be recorded");

    // Verify both events emitted in the right order: Connected (first),
    // ParticipantJoined (always).
    let emit_order: Vec<_> = deps
        .call_log()
        .into_iter()
        .filter_map(|e| match e {
            MockEvent::Emit(label) => Some(label),
            _ => None,
        })
        .collect();
    assert_eq!(
        emit_order,
        vec![
            "GroupCallConnected".to_string(),
            "GroupCallParticipantJoined".to_string()
        ],
        "First-accept must emit Connected before ParticipantJoined"
    );
}

#[tokio::test]
async fn group_accept_second_acceptor_does_not_re_emit_connected() {
    let deps = MockDeps::new();
    let initiator = "aa".repeat(32);
    let p1 = "bb".repeat(32);
    let p2 = "cc".repeat(32);

    // Pre-seed: first acceptor already in the set, status already Active.
    let mut accepted = std::collections::HashSet::new();
    accepted.insert(p1.clone());
    deps.group_registry.insert(GroupCallState {
        call_id: "group-2".into(),
        initiator_pubkey: initiator.clone(),
        kind: 0,
        participants: vec![initiator, p1, p2.clone()],
        accepted,
        our_x25519_secret: Some(StaticSecret::random_from_rng(rand::rngs::OsRng)),
        call_key: Some([0xAA; 32]),
        status: GroupCallStatus::Active,
    });

    crate::signaling::group_handlers::handle_group_accept_received(
        deps.as_ref(),
        &p2,
        "group-2",
        &p2,
    );

    let emit_order: Vec<_> = deps
        .call_log()
        .into_iter()
        .filter_map(|e| match e {
            MockEvent::Emit(label) => Some(label),
            _ => None,
        })
        .collect();
    assert_eq!(
        emit_order,
        vec!["GroupCallParticipantJoined".to_string()],
        "Second accept emits only ParticipantJoined (not GroupCallConnected again)"
    );
}

// ─── Incoming-call timeout regression: missed-call timer wired ────────

// ─── friend_display_name fallback contract ───────────────────────────
//
// The deps trait's `friend_display_name` MUST return the raw value
// (possibly empty) — NOT pre-fallback to short_pubkey. The crate
// handlers detect empty + apply `short_pubkey(initiator_pubkey)` as
// the fallback (which may differ from `peer_pubkey`). If the adapter
// pre-applies a fallback, the crate's "use initiator_pubkey" branch
// is bypassed.

struct EmptyNameDeps(Arc<MockDeps>);

#[async_trait]
impl CallSignalingDeps for EmptyNameDeps {
    fn owner_key(&self) -> Result<String, CallError> {
        self.0.owner_key()
    }
    fn identity_secret(&self) -> Result<[u8; 32], CallError> {
        self.0.identity_secret()
    }
    fn registry(&self) -> Arc<dyn CallRegistry> {
        self.0.registry()
    }
    fn group_registry(&self) -> Arc<dyn GroupCallRegistry> {
        self.0.group_registry()
    }
    fn is_peer_temp_muted(&self, p: &str) -> bool {
        self.0.is_peer_temp_muted(p)
    }
    /// Returns empty — simulates "friend not found in friends map".
    fn friend_display_name(&self, _peer: &str) -> String {
        String::new()
    }
    async fn send_to_peer(&self, p: &str, msg: MessagePayload) -> Result<(), CallError> {
        self.0.send_to_peer(p, msg).await
    }
    async fn start_voice_session(
        &self,
        c: &str,
        p: &str,
        k: [u8; 32],
        kind: CallKind,
    ) -> Result<(), CallError> {
        self.0.start_voice_session(c, p, k, kind).await
    }
    async fn shutdown_voice_session(&self) {
        self.0.shutdown_voice_session().await;
    }
    fn voice_active(&self) -> bool {
        self.0.voice_active()
    }
    fn pre_stage_voice_channel(&self) {
        self.0.pre_stage_voice_channel();
    }
    fn persist_missed_call(&self, c: &str, p: &str, k: CallKind, e: u64) {
        self.0.persist_missed_call(c, p, k, e);
    }
    fn surface_window_for_call(&self, c: &str) {
        self.0.surface_window_for_call(c);
    }
    fn emit_event(&self, e: CallSignalEvent) {
        self.0.emit_event(e);
    }
    fn register_background_handle(&self, h: tokio::task::JoinHandle<()>) {
        self.0.register_background_handle(h);
    }
    fn spawn_incoming_call_timeout(&self, c: String, p: String, k: CallKind, e: u64) {
        self.0.spawn_incoming_call_timeout(c, p, k, e);
    }
    fn spawn_dialing_call_timeout(&self, c: String, p: String, k: CallKind, e: u64) {
        self.0.spawn_dialing_call_timeout(c, p, k, e);
    }
}

#[tokio::test]
async fn empty_friend_display_name_falls_back_to_initiator_pubkey() {
    let inner = MockDeps::new();
    let deps = Arc::new(EmptyNameDeps(Arc::clone(&inner)));

    let sender = "bb".repeat(32);
    // Use a DIFFERENT initiator_pubkey from sender to verify the
    // fallback picks initiator_pubkey (the crate's intent).
    let initiator = "ff".repeat(32);
    let initiator_x = peer_x25519_pub_bytes();

    crate::signaling::handlers::handle_incoming_invite(
        deps.as_ref(),
        &sender,
        "call-fb",
        0,
        &initiator,
        &initiator_x,
        12_345_678,
    )
    .await;

    // Find the emitted IncomingCall and check its display_name.
    let incoming = inner
        .emitted_events()
        .into_iter()
        .find_map(|e| match e {
            CallSignalEvent::IncomingCall {
                from_display_name, ..
            } => Some(from_display_name),
            _ => None,
        })
        .expect("IncomingCall must be emitted");

    // Expected: short_pubkey(initiator_pubkey) — "ff…" prefix, NOT
    // sender_hex's "bb…" prefix.
    assert!(
        incoming.starts_with("ff"),
        "Empty friend_display_name must fall back to short_pubkey(initiator_pubkey). \
         Got: {incoming:?} — expected to start with 'ff' (initiator), not 'bb' (sender)."
    );
}

#[tokio::test]
async fn handle_incoming_invite_arms_incoming_timeout() {
    let deps = MockDeps::new();
    let initiator = "bb".repeat(32);

    let initiator_x = peer_x25519_pub_bytes();
    crate::signaling::handlers::handle_incoming_invite(
        deps.as_ref(),
        &initiator,
        "call-incoming-1",
        0, // CallKind::Audio
        &initiator,
        &initiator_x,
        12_345_678, // expires_at_ms
    )
    .await;

    let timeout_arm = deps
        .call_log()
        .into_iter()
        .find(|e| matches!(e, MockEvent::SpawnIncomingTimeout { .. }));
    let Some(MockEvent::SpawnIncomingTimeout {
        call_id,
        peer_pubkey,
        kind,
        expires_at_ms,
    }) = timeout_arm
    else {
        panic!(
            "handle_incoming_invite MUST arm spawn_incoming_call_timeout — \
                otherwise missed-call notifications never fire. Call log: {:?}",
            deps.call_log()
        );
    };
    assert_eq!(call_id, "call-incoming-1");
    assert_eq!(peer_pubkey, "bb".repeat(32));
    assert_eq!(kind, CallKind::Audio);
    assert_eq!(expires_at_ms, 12_345_678);
}

#[tokio::test]
async fn group_accept_from_non_invitee_ignored() {
    let deps = MockDeps::new();
    let initiator = "aa".repeat(32);
    let p1 = "bb".repeat(32);
    let outsider = "ee".repeat(32);

    deps.group_registry.insert(GroupCallState {
        call_id: "group-3".into(),
        initiator_pubkey: initiator.clone(),
        kind: 0,
        participants: vec![initiator, p1],
        accepted: std::collections::HashSet::new(),
        our_x25519_secret: Some(StaticSecret::random_from_rng(rand::rngs::OsRng)),
        call_key: Some([0xAA; 32]),
        status: GroupCallStatus::Outgoing,
    });

    crate::signaling::group_handlers::handle_group_accept_received(
        deps.as_ref(),
        &outsider,
        "group-3",
        &outsider,
    );

    let snapshot = deps.group_registry.snapshot("group-3").unwrap();
    assert_eq!(
        snapshot.status,
        GroupCallStatus::Outgoing,
        "Non-invitee accept must NOT promote status"
    );
    assert_eq!(snapshot.accepted_count, 0);
    let emit_count = deps
        .call_log()
        .iter()
        .filter(|e| matches!(e, MockEvent::Emit(_)))
        .count();
    assert_eq!(emit_count, 0, "Non-invitee accept must emit nothing");
}
