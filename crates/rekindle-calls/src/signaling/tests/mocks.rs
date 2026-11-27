//! Shared mock deps/registries for the signaling regression suites.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use rekindle_codec::capnp_codec::direct::DirectBody;
use rekindle_types::group_dm::{GroupCallRoster, GroupDmRosterMember};
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
    /// Every body handed to `send_to_peer`, with its peer.
    pub(super) sent: Vec<(String, DirectBody)>,
    /// Every `CallEnd` committed to the outbox: (peer, body, deadline).
    pub(super) committed: Vec<(String, DirectBody, u64)>,
    /// Every `CallInvite` committed: (peer, call id, deadline).
    pub(super) invites: Vec<(String, String, u64)>,
    /// Every `settle_call_invite`: (call id, answered).
    pub(super) settled: Vec<(String, bool)>,
    /// When set, `commit_call_end` never completes (a send that hangs).
    pub(super) commit_hangs: bool,
    pub(super) shutdowns: usize,
    pub(super) now_ms: u64,
    pub(super) last_inbound_ms: Option<u64>,
    /// `spawn_consent_watch` calls: (call id, since).
    pub(super) consent_watches: Vec<(String, u64)>,
}

impl Default for MockState {
    fn default() -> Self {
        Self {
            call_log: Vec::new(),
            emitted: Vec::new(),
            start_voice_session_result: Ok(()),
            sent: Vec::new(),
            committed: Vec::new(),
            commit_hangs: false,
            invites: Vec::new(),
            settled: Vec::new(),
            shutdowns: 0,
            now_ms: 1_000_000,
            last_inbound_ms: None,
            consent_watches: Vec::new(),
        }
    }
}

pub(super) struct MockRegistry {
    inner: Mutex<HashMap<String, CallState>>,
    ended: Mutex<std::collections::HashSet<String>>,
}
impl CallRegistry for MockRegistry {
    fn insert(&self, call: CallState) {
        self.inner.lock().insert(call.call_id.clone(), call);
    }
    fn get(&self, call_id: &str) -> Option<CallState> {
        self.inner.lock().get(call_id).cloned()
    }
    fn remove(&self, call_id: &str) -> Option<CallState> {
        let removed = self.inner.lock().remove(call_id);
        if removed.is_some() {
            self.ended.lock().insert(call_id.to_string());
        }
        removed
    }
    fn contains(&self, call_id: &str) -> bool {
        self.inner.lock().contains_key(call_id)
    }
    fn ended_recently(&self, call_id: &str) -> bool {
        self.ended.lock().contains(call_id)
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
    fn list_ids(&self) -> Vec<String> {
        self.inner.lock().keys().cloned().collect()
    }

    fn snapshot(&self, call_id: &str) -> Option<GroupCallSnapshot> {
        let g = self.inner.lock();
        let c = g.get(call_id)?;
        Some(GroupCallSnapshot {
            call_id: c.call_id.clone(),
            group_id: c.group_id.clone(),
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
    /// Which call's media runs (plan CE2), as the desktop adapter keeps it.
    pub(super) media: crate::signaling::CallMediaOwner,
    pub(super) friend_names: Mutex<HashMap<String, String>>,
    owner_key: String,
    pub(super) own_name: Mutex<String>,
    identity_secret: [u8; 32],
    /// Our group DMs' rosters, by group id (`group_roster`).
    pub(super) group_rosters: Mutex<HashMap<String, Vec<GroupDmRosterMember>>>,
    /// Groups we hold only an unanswered invite to.
    pub(super) invited_groups: Mutex<HashSet<String>>,
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
                ended: Mutex::new(std::collections::HashSet::new()),
            }),
            group_registry: Arc::new(MockGroupRegistry {
                inner: Mutex::new(HashMap::new()),
            }),
            media: crate::signaling::CallMediaOwner::default(),
            friend_names: Mutex::new(HashMap::new()),
            owner_key: "aa".repeat(32),
            own_name: Mutex::new("My profile".into()),
            group_rosters: Mutex::new(HashMap::new()),
            invited_groups: Mutex::new(HashSet::new()),
            identity_secret: [0xCC; 32],
        })
    }

    /// A mock whose owner key and identity secret are a real pair, so
    /// group call keys it wraps unwrap at another such mock.
    pub(super) fn with_identity(secret: [u8; 32]) -> Arc<Self> {
        let mut deps = Arc::into_inner(Self::new()).expect("fresh mock");
        deps.owner_key = rekindle_crypto::Identity::from_secret_bytes(&secret).public_key_hex();
        deps.identity_secret = secret;
        Arc::new(deps)
    }

    pub(super) fn call_log(&self) -> Vec<MockEvent> {
        self.state.lock().call_log.clone()
    }
    pub(super) fn emitted_events(&self) -> Vec<CallSignalEvent> {
        self.state.lock().emitted.clone()
    }
    /// The mock's whole state, to set inputs and read what it recorded.
    pub(super) fn with<R>(&self, f: impl FnOnce(&mut MockState) -> R) -> R {
        f(&mut self.state.lock())
    }
}

#[async_trait]
impl CallSignalingDeps for MockDeps {
    fn owner_key(&self) -> Result<String, CallError> {
        Ok(self.owner_key.clone())
    }
    fn own_display_name(&self) -> Result<String, CallError> {
        Ok(self.own_name.lock().clone())
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
    async fn group_roster(&self, group_id: &str) -> Result<GroupCallRoster, CallError> {
        if let Some(keys) = self.group_rosters.lock().get(group_id) {
            return Ok(GroupCallRoster::Held(keys.clone()));
        }
        if self.invited_groups.lock().contains(group_id) {
            return Ok(GroupCallRoster::Invited);
        }
        Ok(GroupCallRoster::Unknown)
    }
    fn is_peer_temp_muted(&self, _peer: &str) -> bool {
        false
    }
    fn friend_display_name(&self, peer: &str) -> Option<String> {
        self.friend_names.lock().get(peer).cloned()
    }
    fn local_video_decode_codecs(&self) -> Vec<String> {
        vec!["vp9".to_string()]
    }
    async fn send_to_peer(&self, peer: &str, body: DirectBody) -> Result<(), CallError> {
        self.state.lock().sent.push((peer.to_string(), body));
        Ok(())
    }
    async fn commit_call_end(
        &self,
        peer: &str,
        body: DirectBody,
        deadline_ms: u64,
    ) -> Result<(), CallError> {
        let hangs = {
            let mut s = self.state.lock();
            s.committed.push((peer.to_string(), body, deadline_ms));
            s.commit_hangs
        };
        if hangs {
            std::future::pending::<()>().await;
        }
        Ok(())
    }
    async fn commit_call_invite(
        &self,
        peer: &str,
        call_id: &str,
        _body: DirectBody,
        deadline_ms: u64,
    ) -> Result<(), CallError> {
        self.state
            .lock()
            .invites
            .push((peer.to_string(), call_id.to_string(), deadline_ms));
        Ok(())
    }
    async fn settle_call_invite(&self, call_id: &str, answered: bool) {
        self.state
            .lock()
            .settled
            .push((call_id.to_string(), answered));
    }
    fn now_ms(&self) -> u64 {
        self.state.lock().now_ms
    }
    fn last_inbound_media_ms(&self, _call_id: &str) -> Option<u64> {
        self.state.lock().last_inbound_ms
    }
    fn spawn_consent_watch(&self, call_id: String, since_ms: u64) {
        self.state.lock().consent_watches.push((call_id, since_ms));
    }
    async fn start_voice_session(
        &self,
        call_id: &str,
        _peer: &str,
        kind: CallKind,
    ) -> Result<(), CallError> {
        let mut s = self.state.lock();
        s.call_log.push(MockEvent::StartVoiceSession {
            call_id: call_id.to_string(),
            kind,
        });
        let result = std::mem::replace(&mut s.start_voice_session_result, Ok(()));
        if result.is_ok() {
            // As the desktop adapter: another call's media is torn down
            // first, then this call owns the one session.
            self.media.claim(call_id);
        }
        result
    }
    async fn shutdown_voice_session(&self, call_id: &str) {
        self.state.lock().shutdowns += 1;
        // As the desktop adapter: only the owning call's end stops it.
        self.media.release(call_id);
    }
    fn voice_active(&self) -> bool {
        false
    }
    fn pre_stage_voice_channel(&self) {
        self.state.lock().call_log.push(MockEvent::PreStage);
    }
    fn persist_missed_call(&self, _: &str, _: &str, _: CallKind, _: u64) {}
    fn surface_window_for_call(&self, _: &str) {}
    fn present_active_call(&self, _: &str) {}
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
            CallSignalEvent::GroupCallStarted { .. } => "GroupCallStarted",
            CallSignalEvent::GroupCallConnected { .. } => "GroupCallConnected",
            CallSignalEvent::GroupCallParticipantJoined { .. } => "GroupCallParticipantJoined",
            CallSignalEvent::GroupCallParticipantLeft { .. } => "GroupCallParticipantLeft",
            CallSignalEvent::GroupCallEnded { .. } => "GroupCallEnded",
        };
        s.call_log.push(MockEvent::Emit(label.to_string()));
        s.emitted.push(event);
    }
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

/// Set up a CallState in `status` with `peer_hex`.
pub(super) fn seed_call(deps: &MockDeps, call_id: &str, peer_hex: &str, status: CallStatus) {
    seed_outgoing_call(deps, call_id, peer_hex, CallKind::Audio);
    let mut call = deps.registry.get(call_id).expect("seeded");
    call.status = status;
    deps.registry.insert(call);
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
        media_secret: None,
        media_sender: std::sync::Arc::new(rekindle_secrets::sframe::SframeSender::fresh()),
        peer_video_decode_codecs: Vec::new(),
    });
}

pub(super) fn peer_x25519_pub_bytes() -> [u8; 32] {
    let secret = StaticSecret::random_from_rng(rand::rngs::OsRng);
    PublicKey::from(&secret).to_bytes()
}
