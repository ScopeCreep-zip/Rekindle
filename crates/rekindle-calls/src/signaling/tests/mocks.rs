//! Shared mock deps/registries for the signaling regression suites.

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
pub(super) enum MockEvent {
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

pub(super) struct MockState {
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

pub(super) struct MockRegistry {
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

pub(super) struct MockGroupRegistry {
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

pub(super) struct MockDeps {
    state: Mutex<MockState>,
    pub(super) registry: Arc<MockRegistry>,
    pub(super) group_registry: Arc<MockGroupRegistry>,
    owner_key: String,
    identity_secret: [u8; 32],
}

impl MockDeps {
    pub(super) fn new() -> Arc<Self> {
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

    pub(super) fn call_log(&self) -> Vec<MockEvent> {
        self.state.lock().call_log.clone()
    }
    pub(super) fn emitted_events(&self) -> Vec<CallSignalEvent> {
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
    fn local_video_decode_codecs(&self) -> Vec<String> {
        vec!["vp9".to_string()]
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
pub(super) fn seed_outgoing_call(deps: &MockDeps, call_id: &str, peer_hex: &str, kind: CallKind) {
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
        peer_video_decode_codecs: Vec::new(),
    });
}

pub(super) fn peer_x25519_pub_bytes() -> [u8; 32] {
    let secret = StaticSecret::random_from_rng(rand::rngs::OsRng);
    PublicKey::from(&secret).to_bytes()
}
