//! W16.6 — 1:1 call signaling operations.
//!
//! [`CallRuntime`] is the orchestration layer that drives the
//! pure-logic [`CallStateMachine`] (W16.5). Each public method on the
//! runtime represents either a local user action (`start_dm_call`,
//! `accept_dm_call`, `decline_dm_call`, `end_dm_call`) or a mid-call
//! state ping (`send_call_media_state`, `send_call_reaction`).
//!
//! Each method:
//! 1. Constructs a [`CallEvent`] for the change.
//! 2. Drives the state machine via `apply` (returns `Vec<Effect>`).
//! 3. Interprets effects: serializes envelope sends through
//!    [`EnvelopeQueue`], spawns timers, persists state via
//!    [`EnvelopeStore`], emits notifications via [`SharedState::notify`],
//!    and delegates voice-session start/stop to [`VoiceSessionLauncher`].
//!
//! The receive-side dispatch (W16.7) and ring timer integration also
//! call back into this runtime via the same `apply` + interpret-effects
//! shape, which keeps every event flow through the same pipeline.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use rekindle_calls::{
    fresh_keypair, CallEvent, CallKind, CallStateMachine, CallStatus, Effect,
};
use rekindle_types::notification::TransportNotification;
use rekindle_utils::timestamp_ms;
use tokio::task::JoinHandle;
use tracing::{debug, warn};

use crate::envelope_queue::EnvelopeQueue;
use crate::envelope_store::{EnvelopeKind, EnvelopeStore, PersistedCallState};
use crate::payload::dm::{serialize_dm, DmPayload};
use crate::shared::SharedState;

/// Default ring duration matches arch §10.10 — 30 s window for both
/// caller-side dialing and receiver-side incoming.
pub const RING_DURATION_MS: u64 = 30_000;

/// W16.5b — timeout for the `app_call` CallInvite handshake. Veilid's
/// `network.rpc.timeout_ms` defaults to 5 s; through private routes
/// `rpc_processor/mod.rs:1300-1303` doubles it to 10 s. We use 10 s
/// to give the receiver's `on_call` handler enough budget for the
/// state-machine + persist operations even on slow disks. If the
/// receiver is unreachable, the timeout fires fast — well under the
/// 30 s ring window — and the caller's UI surfaces "peer unreachable"
/// immediately via `CallUnreachable`.
pub const CALL_INVITE_RPC_TIMEOUT_MS: u64 = 10_000;

/// Hook the runtime calls to bring up / tear down the audio + jitter
/// pipeline for a call. Implementer is shell-specific (Tauri uses
/// cpal-backed `rekindle-voice`; CLI/daemon could no-op or use a
/// different backend).
///
/// Errors propagate back through the state machine as a
/// `VoiceTransportDown` event so the call cleans up consistently.
#[async_trait]
pub trait VoiceSessionLauncher: Send + Sync {
    async fn start_voice_session(
        &self,
        call_id: &str,
        peer: &str,
        kind: CallKind,
        call_key: [u8; 32],
    ) -> Result<(), String>;

    async fn stop_voice_session(&self, call_id: &str, reason: &str);
}

/// No-op launcher useful for tests and headless deployments that don't
/// run audio. Logs every call but doesn't actually start audio.
pub struct NoopVoiceSessionLauncher;

#[async_trait]
impl VoiceSessionLauncher for NoopVoiceSessionLauncher {
    async fn start_voice_session(
        &self,
        call_id: &str,
        peer: &str,
        _kind: CallKind,
        _call_key: [u8; 32],
    ) -> Result<(), String> {
        debug!(call_id, peer, "NoopVoiceSessionLauncher::start_voice_session (no audio)");
        Ok(())
    }

    async fn stop_voice_session(&self, call_id: &str, reason: &str) {
        debug!(call_id, reason, "NoopVoiceSessionLauncher::stop_voice_session");
    }
}

/// Errors surfaced by the call runtime to callers.
#[derive(Debug, thiserror::Error)]
pub enum CallError {
    #[error("identity not initialized")]
    NoIdentity,
    #[error("call not found: {0}")]
    NotFound(String),
    #[error("invalid state: {0}")]
    InvalidState(String),
    #[error("queue: {0}")]
    Queue(String),
    #[error("store: {0}")]
    Store(String),
    #[error("serialize: {0}")]
    Serialize(String),
}

/// Orchestrates the 1:1 call lifecycle. Cloned cheaply (internal `Arc`s).
#[derive(Clone)]
pub struct CallRuntime {
    inner: Arc<Inner>,
}

struct Inner {
    state_machine: Mutex<CallStateMachine>,
    queue: EnvelopeQueue,
    store: Arc<dyn EnvelopeStore>,
    notifications: Arc<SharedState>,
    voice_launcher: Arc<dyn VoiceSessionLauncher>,
    /// Per-call tokio task handles for ring timers. Keyed by call_id.
    timers: Mutex<HashMap<String, JoinHandle<()>>>,
    owner_key: String,
}

impl CallRuntime {
    /// Construct a new runtime. The runtime borrows the queue + store +
    /// notifications channel from the transport node; owner_key is the
    /// local identity's hex pubkey (used for store scoping).
    pub fn new(
        queue: EnvelopeQueue,
        store: Arc<dyn EnvelopeStore>,
        notifications: Arc<SharedState>,
        voice_launcher: Arc<dyn VoiceSessionLauncher>,
        owner_key: impl Into<String>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                state_machine: Mutex::new(CallStateMachine::new()),
                queue,
                store,
                notifications,
                voice_launcher,
                timers: Mutex::new(HashMap::new()),
                owner_key: owner_key.into(),
            }),
        }
    }

    // ── Local user actions ──────────────────────────────────────────

    /// User clicked Voice/Video Call. Generates a fresh ephemeral X25519
    /// keypair, creates a call_id, drives the state machine to
    /// Outgoing, and queues the CallInvite for delivery. Returns the
    /// call_id.
    pub async fn start_dm_call(
        &self,
        peer: &str,
        peer_display_name: &str,
        kind: CallKind,
    ) -> Result<String, CallError> {
        let call_id = generate_call_id();
        let (sk, pk) = fresh_keypair();
        let now = timestamp_ms();
        let expires_at_ms = now + RING_DURATION_MS;

        let event = CallEvent::LocalStartCall {
            call_id: call_id.clone(),
            peer: peer.to_string(),
            peer_display_name: peer_display_name.to_string(),
            kind,
            my_x25519_secret: sk,
            my_x25519_pub: pk,
            expires_at_ms,
            started_at_ms: now,
        };

        let effects = self.inner.state_machine.lock().apply(event);
        self.interpret_effects(effects).await;
        Ok(call_id)
    }

    /// User clicked Accept on the IncomingCallModal. Generates the
    /// receiver's X25519 keypair, drives the state machine to
    /// Connecting, derives the shared call_key, starts the voice
    /// session, and queues the CallAccept envelope.
    pub async fn accept_dm_call(&self, call_id: &str) -> Result<(), CallError> {
        // Validate the call exists and is Incoming before generating
        // keys.
        {
            let sm = self.inner.state_machine.lock();
            let Some(state) = sm.get(call_id) else {
                return Err(CallError::NotFound(call_id.into()));
            };
            if !matches!(state.status, CallStatus::Incoming) {
                return Err(CallError::InvalidState(format!(
                    "call {call_id} is not Incoming (status={:?})",
                    state.status
                )));
            }
        }

        let (sk, pk) = fresh_keypair();
        let event = CallEvent::LocalAccept {
            call_id: call_id.into(),
            my_x25519_secret: sk,
            my_x25519_pub: pk,
        };
        let effects = self.inner.state_machine.lock().apply(event);
        self.interpret_effects(effects).await;
        Ok(())
    }

    /// User clicked Decline on the IncomingCallModal.
    pub async fn decline_dm_call(
        &self,
        call_id: &str,
        reason: &str,
    ) -> Result<(), CallError> {
        let event = CallEvent::LocalDecline {
            call_id: call_id.into(),
            reason: reason.into(),
        };
        let effects = self.inner.state_machine.lock().apply(event);
        if effects.is_empty() {
            return Err(CallError::NotFound(call_id.into()));
        }
        self.interpret_effects(effects).await;
        Ok(())
    }

    /// User clicked Hangup or Cancel. Works in any state — Outgoing
    /// (cancel before peer accepts), Connecting / Active (mid-call
    /// hangup).
    pub async fn end_dm_call(
        &self,
        call_id: &str,
        reason: &str,
    ) -> Result<(), CallError> {
        let event = CallEvent::LocalCancel {
            call_id: call_id.into(),
            reason: reason.into(),
        };
        let effects = self.inner.state_machine.lock().apply(event);
        if effects.is_empty() {
            return Err(CallError::NotFound(call_id.into()));
        }
        self.interpret_effects(effects).await;
        Ok(())
    }

    /// Mid-call: peer should learn that our mic / camera / screen
    /// state changed. Direct send via queue — no state machine event.
    pub async fn send_call_media_state(
        &self,
        call_id: &str,
        audio: bool,
        video: bool,
        screen: bool,
    ) -> Result<(), CallError> {
        let peer = self
            .inner
            .state_machine
            .lock()
            .get(call_id)
            .map(|s| s.peer_pubkey.clone())
            .ok_or_else(|| CallError::NotFound(call_id.into()))?;
        let payload = DmPayload::CallMediaState {
            call_id: call_id.into(),
            audio,
            video,
            screen,
            timestamp_ms: timestamp_ms(),
        };
        let bytes = serialize_dm(&payload).map_err(|e| CallError::Serialize(e.to_string()))?;
        self.inner
            .queue
            .send(&peer, bytes, EnvelopeKind::CallMediaState, Some(call_id))
            .await
            .map_err(|e| CallError::Queue(e.to_string()))
    }

    /// Mid-call: emoji reaction. Same direct-send shape as
    /// `send_call_media_state`.
    pub async fn send_call_reaction(
        &self,
        call_id: &str,
        emoji: &str,
    ) -> Result<(), CallError> {
        let peer = self
            .inner
            .state_machine
            .lock()
            .get(call_id)
            .map(|s| s.peer_pubkey.clone())
            .ok_or_else(|| CallError::NotFound(call_id.into()))?;
        let payload = DmPayload::CallReaction {
            call_id: call_id.into(),
            emoji: emoji.into(),
            timestamp_ms: timestamp_ms(),
        };
        let bytes = serialize_dm(&payload).map_err(|e| CallError::Serialize(e.to_string()))?;
        self.inner
            .queue
            .send(&peer, bytes, EnvelopeKind::CallReaction, Some(call_id))
            .await
            .map_err(|e| CallError::Queue(e.to_string()))
    }

    // ── Inbound event entry points (called from W16.7's dispatch) ───
    //
    // These mirror the local actions but are driven by inbound
    // envelopes / timer firings. The W16.7 receive dispatch pulls
    // (call_id, sender, payload) out of the wire envelope, builds the
    // appropriate CallEvent, and calls the matching dispatch helper.

    /// W16.7 entry: an inbound CallInvite arrived. Drives state
    /// machine to Incoming.
    pub async fn handle_invite_received(
        &self,
        call_id: String,
        from: String,
        from_display_name: String,
        kind: CallKind,
        peer_x25519_pub: [u8; 32],
        expires_at_ms: u64,
    ) {
        let event = CallEvent::InviteReceived {
            call_id,
            from,
            from_display_name,
            kind,
            peer_x25519_pub,
            expires_at_ms,
            received_at_ms: timestamp_ms(),
        };
        let effects = self.inner.state_machine.lock().apply(event);
        self.interpret_effects(effects).await;
    }

    /// W16.7 entry: an inbound CallAccept arrived (caller-side).
    pub async fn handle_accept_received(
        &self,
        call_id: String,
        from: String,
        peer_x25519_pub: [u8; 32],
    ) {
        let event = CallEvent::AcceptReceived {
            call_id,
            from,
            peer_x25519_pub,
        };
        let effects = self.inner.state_machine.lock().apply(event);
        self.interpret_effects(effects).await;
    }

    /// W16.7 entry: an inbound CallDecline arrived (caller-side).
    pub async fn handle_decline_received(&self, call_id: String, reason: String) {
        let event = CallEvent::DeclineReceived { call_id, reason };
        let effects = self.inner.state_machine.lock().apply(event);
        self.interpret_effects(effects).await;
    }

    /// W16.7 entry: an inbound CallRinging arrived (caller-side
    /// alerting hint).
    pub async fn handle_ringing_received(&self, call_id: String) {
        let event = CallEvent::RingingReceived { call_id };
        let effects = self.inner.state_machine.lock().apply(event);
        self.interpret_effects(effects).await;
    }

    /// W16.7 entry: an inbound CallEnd arrived from either side.
    pub async fn handle_end_received(&self, call_id: String, reason: String) {
        let event = CallEvent::EndReceived { call_id, reason };
        let effects = self.inner.state_machine.lock().apply(event);
        self.interpret_effects(effects).await;
    }

    /// W16.7 entry: voice transport finished bringing up audio.
    pub async fn handle_voice_transport_up(&self, call_id: String) {
        let event = CallEvent::VoiceTransportUp { call_id };
        let effects = self.inner.state_machine.lock().apply(event);
        self.interpret_effects(effects).await;
    }

    /// W16.7 entry: voice transport failed mid-call.
    pub async fn handle_voice_transport_down(&self, call_id: String, reason: String) {
        let event = CallEvent::VoiceTransportDown { call_id, reason };
        let effects = self.inner.state_machine.lock().apply(event);
        self.interpret_effects(effects).await;
    }

    /// True if there's already an outgoing call to this peer (used by
    /// W16.7's glare detection — on inbound CallInvite for a peer we're
    /// already calling, lower-pubkey-wins resolution kicks in).
    pub fn has_outgoing_to(&self, peer: &str) -> bool {
        self.inner.state_machine.lock().has_outgoing_to(peer)
    }

    /// W16.8 — crash recovery. Rehydrates persisted Outgoing/Incoming
    /// call state from [`EnvelopeStore`] and either:
    /// - **Resumes** the call if the ring window is still open (re-emit
    ///   `CallStarted`/`IncomingCall` notification, spawn a fresh
    ///   ring timer for the remaining window).
    /// - **Drops** the row as a missed call if the ring already
    ///   expired (emit `CallTimedOut`/`CallMissed`, delete the row).
    ///
    /// Active call state is intentionally NOT persisted (matches Signal
    /// + Discord — voice transport state is process-bound), so this
    /// method only restores Outgoing/Incoming. After rehydration runs
    /// a single immediate envelope-queue retry tick so any in-flight
    /// CallInvite/CallAccept envelopes that were due during the
    /// downtime fire fast on launch.
    ///
    /// Call once at app startup, after `TransportNode::start` and
    /// before accepting new user actions.
    pub async fn recover(&self) {
        let states = match self
            .inner
            .store
            .load_active_calls(&self.inner.owner_key)
            .await
        {
            Ok(s) => s,
            Err(e) => {
                warn!(error = %e, "recover_active_calls: load_active_calls failed");
                return;
            }
        };
        debug!(count = states.len(), "CallRuntime::recover: rehydrating");

        let now = timestamp_ms();
        for persisted in states {
            self.recover_one(persisted, now).await;
        }

        // Run an immediate retry tick so any envelopes whose
        // `next_retry_at_ms` already passed during the downtime fire
        // now instead of waiting for the next tick.
        self.inner.queue.run_retry_tick().await;
    }

    async fn recover_one(&self, persisted: PersistedCallState, now_ms: u64) {
        let call_id = persisted.call_id.clone();
        let kind = parse_kind(&persisted.kind).unwrap_or(CallKind::Audio);
        let Some(status) = parse_status(&persisted.status) else {
            warn!(
                call_id,
                status = %persisted.status,
                "recover: unknown persisted status, deleting"
            );
            let _ = self
                .inner
                .store
                .delete_active_call(&self.inner.owner_key, &call_id)
                .await;
            return;
        };

        // Only Outgoing/Incoming are valid persisted statuses (W16.8
        // never persists Connecting/Active/Missed).
        if !matches!(status, CallStatus::Outgoing | CallStatus::Incoming) {
            warn!(
                call_id,
                ?status,
                "recover: unexpected persisted status, deleting"
            );
            let _ = self
                .inner
                .store
                .delete_active_call(&self.inner.owner_key, &call_id)
                .await;
            return;
        }

        // Expired during downtime — emit timeout/missed, delete.
        if persisted.expires_at_ms <= now_ms {
            let _ = self
                .inner
                .store
                .delete_active_call(&self.inner.owner_key, &call_id)
                .await;
            match status {
                CallStatus::Outgoing => {
                    self.inner
                        .notifications
                        .notify(&TransportNotification::CallTimedOut {
                            call_id: call_id.clone(),
                        });
                }
                CallStatus::Incoming => {
                    self.inner
                        .notifications
                        .notify(&TransportNotification::CallMissed {
                            call_id: call_id.clone(),
                            from: persisted.peer_pubkey.clone(),
                        });
                }
                _ => {}
            }
            return;
        }

        // Still within the ring window — rehydrate and resume.
        let my_secret = persisted
            .my_x25519_secret
            .as_deref()
            .and_then(|b| <[u8; 32]>::try_from(b).ok())
            .map(x25519_dalek::StaticSecret::from);
        let peer_pub = persisted
            .peer_x25519_pub
            .as_deref()
            .and_then(|b| <[u8; 32]>::try_from(b).ok());

        let state = rekindle_calls::CallState {
            call_id: call_id.clone(),
            peer_pubkey: persisted.peer_pubkey.clone(),
            kind,
            status,
            expires_at_ms: persisted.expires_at_ms,
            my_x25519_secret: my_secret,
            peer_x25519_pub: peer_pub,
            // call_key is not persisted (held only in-memory once
            // derived); a recovered Outgoing/Incoming call hasn't
            // accepted yet, so call_key is None either way.
            call_key: None,
        };
        self.inner.state_machine.lock().rehydrate(state);

        // Spawn the matching ring timer for the remaining window.
        match status {
            CallStatus::Outgoing => {
                self.spawn_timeout(
                    call_id.clone(),
                    persisted.expires_at_ms,
                    TimerKind::Dialing,
                );
                // Re-emit CallStarted so the UI re-mounts the
                // OutgoingCallPanel. Display name is unknown
                // post-restart (the friend-list resolver runs
                // shell-side; the runtime can't look it up here).
                self.inner.notifications.notify(
                    &TransportNotification::CallStarted {
                        call_id,
                        kind: kind_str(kind).into(),
                        peer_key: persisted.peer_pubkey,
                        peer_display_name: String::new(),
                        expires_at_ms: persisted.expires_at_ms,
                        started_at_ms: persisted.inserted_at_ms,
                        status: "calling".into(),
                    },
                );
            }
            CallStatus::Incoming => {
                self.spawn_timeout(
                    call_id.clone(),
                    persisted.expires_at_ms,
                    TimerKind::Incoming,
                );
                self.inner.notifications.notify(
                    &TransportNotification::IncomingCall {
                        call_id,
                        kind: kind_str(kind).into(),
                        from: persisted.peer_pubkey,
                        display_name: String::new(),
                        expires_at_ms: persisted.expires_at_ms,
                        received_at_ms: persisted.inserted_at_ms,
                        is_group: false,
                    },
                );
            }
            _ => {}
        }
    }

    /// W16.7 — route a [`DmPayload`] received via `InboundHandler::on_dm`
    /// to the matching `handle_*` method. Returns `true` if the payload
    /// was a call-signaling variant (handled by the runtime); `false` if
    /// it was a non-call DmPayload (caller routes elsewhere — DM body,
    /// friend-add, presence, etc.).
    ///
    /// Implementers' typical shape:
    /// ```ignore
    /// async fn on_dm(&self, sender, payload, ts, seq, correlation_id) {
    ///     if !self.seq_tracker.check_and_record(...).await? { return; }
    ///     if self.call_runtime.route_dm_payload(sender, payload, ts).await {
    ///         return;
    ///     }
    ///     // Fall through to friend-add / DM body / etc.
    /// }
    /// ```
    ///
    /// This consumes `payload` (since each handler arm needs ownership
    /// to extract the call_id, x25519 pub, etc.). On false, the caller
    /// has lost the original payload — but in practice, on `true` the
    /// runtime handled it; on `false`, the caller would have processed
    /// it themselves anyway. If a caller needs to inspect-then-decide,
    /// they can pattern-match before calling this helper.
    pub async fn route_dm_payload(
        &self,
        sender: &crate::handler::VerifiedSender,
        payload: DmPayload,
    ) -> bool {
        match payload {
            // W16.5b — CallInvite + CallRinging no longer travel via
            // app_message; the wire-level invite-and-ringing handshake
            // uses Veilid `app_call`. Receive-side routes through
            // `handle_inbound_call_invite` from the `on_call` dispatch
            // (subscriptions/dispatch.rs). Caller-side `RingingReceived`
            // is fed back from the synchronous `app_call` reply by
            // `dispatch_call_invite`.
            DmPayload::CallAccept {
                call_id,
                acceptor_x25519_pub,
            } => {
                let Ok(pub_arr) = <[u8; 32]>::try_from(acceptor_x25519_pub.as_slice()) else {
                    warn!(
                        sender = %sender.public_key,
                        "CallAccept: bad x25519 pub length, dropping"
                    );
                    return true;
                };
                self.handle_accept_received(call_id, sender.public_key.clone(), pub_arr)
                    .await;
                true
            }
            DmPayload::CallDecline { call_id, reason } => {
                self.handle_decline_received(call_id, reason).await;
                true
            }
            DmPayload::CallEnd { call_id, reason } => {
                self.handle_end_received(call_id, reason).await;
                true
            }
            DmPayload::CallMediaState {
                call_id,
                audio,
                video,
                screen,
                timestamp_ms,
            } => {
                // Mid-call media state changes don't drive the state
                // machine — they're transparent UI hints. Emit the
                // notification directly. Drop if the call is unknown.
                if self.inner.state_machine.lock().get(&call_id).is_some() {
                    self.inner.notifications.notify(
                        &TransportNotification::CallMediaStateChanged {
                            call_id,
                            audio,
                            video,
                            screen,
                            timestamp_ms,
                        },
                    );
                }
                true
            }
            DmPayload::CallReaction {
                call_id,
                emoji,
                timestamp_ms,
            } => {
                const MAX_EMOJI_BYTES: usize = 32;
                if emoji.len() > MAX_EMOJI_BYTES {
                    debug!(
                        sender = %sender.public_key,
                        bytes = emoji.len(),
                        "CallReaction: oversized emoji, dropping"
                    );
                    return true;
                }
                if self.inner.state_machine.lock().get(&call_id).is_some() {
                    self.inner.notifications.notify(
                        &TransportNotification::CallReactionReceived {
                            call_id,
                            sender: sender.public_key.clone(),
                            emoji,
                            timestamp_ms,
                        },
                    );
                }
                true
            }
            // Group call signaling (W16.13 — separate runtime), DM
            // invite request/reply (W16.10b — uses
            // `EnvelopeQueue::deliver_reply`), and all other non-call
            // DmPayload variants are unhandled by the 1:1 call runtime.
            // The implementer routes them: group calls to their
            // group-call runtime, DM invites via the queue's
            // expect-reply oneshot registry, the rest to friend-add /
            // DM body / presence handlers.
            _ => false,
        }
    }

    // ── Effect interpreter ──────────────────────────────────────────

    /// Process every effect produced by the state machine. Effects
    /// can recursively produce more effects (e.g. a voice-session
    /// start failure produces a `VoiceTransportDown` event whose
    /// effects also need interpretation), handled by re-entry in the
    /// async task. The `'static` future captures `self` by clone (the
    /// runtime is `Arc`-internal so the clone is cheap).
    fn interpret_effects(&self, effects: Vec<Effect>) -> EffectsFuture {
        let runtime = self.clone();
        Box::pin(async move {
            for effect in effects {
                runtime.interpret_one(effect).await;
            }
        })
    }

    async fn interpret_one(&self, effect: Effect) {
        match effect {
            Effect::SendCallInvite {
                recipient,
                call_id,
                offer_kind,
                initiator_x25519_pub,
                expires_at_ms,
            } => {
                // W16.5b — dispatch CallInvite via Veilid `app_call`
                // (5–10 s budget; matches SIP 100-Trying / 180-Ringing).
                // Receiver replies synchronously inside `app_call_reply`
                // with `CallResponse::CallRinging { call_id }`. Failures
                // map to `CallEvent::LocalUnreachable` so the caller's
                // UI surfaces "peer unreachable" within ~10 s instead
                // of waiting for the 30 s ring timer.
                self.dispatch_call_invite(
                    &recipient,
                    &call_id,
                    offer_kind,
                    initiator_x25519_pub,
                    expires_at_ms,
                )
                .await;
            }
            Effect::SendCallAccept {
                recipient,
                call_id,
                acceptor_x25519_pub,
            } => {
                let payload = DmPayload::CallAccept {
                    call_id: call_id.clone(),
                    acceptor_x25519_pub: acceptor_x25519_pub.to_vec(),
                };
                self.queue_call_envelope(
                    &recipient,
                    &payload,
                    EnvelopeKind::CallAccept,
                    &call_id,
                )
                .await;
            }
            Effect::SendCallDecline {
                recipient,
                call_id,
                reason,
            } => {
                let payload = DmPayload::CallDecline {
                    call_id: call_id.clone(),
                    reason,
                };
                self.queue_call_envelope(
                    &recipient,
                    &payload,
                    EnvelopeKind::CallDecline,
                    &call_id,
                )
                .await;
            }
            Effect::SendCallEnd {
                recipient,
                call_id,
                reason,
            } => {
                let payload = DmPayload::CallEnd {
                    call_id: call_id.clone(),
                    reason,
                };
                self.queue_call_envelope(
                    &recipient,
                    &payload,
                    EnvelopeKind::CallEnd,
                    &call_id,
                )
                .await;
            }
            // W16.5b — Effect::SendCallRinging dropped: the CallRinging
            // reply is synthesized synchronously by `on_call` into the
            // app_call_reply payload, not produced as an outbound
            // envelope. The receiver-side state machine no longer emits
            // this effect.
            Effect::StartVoiceSession {
                call_id,
                peer,
                kind,
                call_key,
            } => {
                if let Err(reason) = self
                    .inner
                    .voice_launcher
                    .start_voice_session(&call_id, &peer, kind, call_key)
                    .await
                {
                    warn!(call_id, peer, reason, "voice session start failed; tearing down call");
                    let event = CallEvent::VoiceTransportDown {
                        call_id: call_id.clone(),
                        reason,
                    };
                    let cleanup = self.inner.state_machine.lock().apply(event);
                    // Recursive re-entry: cleanup may itself send a
                    // CallEnd envelope, persist deletion, emit
                    // notification. Avoid stack-overflow by bounding
                    // recursion to one level (cleanup never re-enters
                    // StartVoiceSession).
                    self.interpret_effects(cleanup).await;
                }
            }
            Effect::StopVoiceSession { call_id, reason } => {
                self.inner
                    .voice_launcher
                    .stop_voice_session(&call_id, &reason)
                    .await;
            }
            Effect::SpawnDialingTimer {
                call_id,
                expires_at_ms,
            } => {
                self.spawn_timeout(call_id, expires_at_ms, TimerKind::Dialing);
            }
            Effect::SpawnIncomingTimer {
                call_id,
                expires_at_ms,
            } => {
                self.spawn_timeout(call_id, expires_at_ms, TimerKind::Incoming);
            }
            Effect::CancelTimer { call_id } => {
                if let Some(handle) = self.inner.timers.lock().remove(&call_id) {
                    handle.abort();
                }
            }
            Effect::PersistCallState {
                call_id,
                peer_pubkey,
                kind,
                status,
                expires_at_ms,
                my_x25519_secret,
                peer_x25519_pub,
            } => {
                let state = PersistedCallState {
                    owner_key: self.inner.owner_key.clone(),
                    call_id,
                    peer_pubkey,
                    kind: kind_str(kind).into(),
                    status: status_str(status).into(),
                    expires_at_ms,
                    my_x25519_secret: my_x25519_secret.map(|b| b.to_vec()),
                    peer_x25519_pub: peer_x25519_pub.map(|b| b.to_vec()),
                    group_participants: vec![],
                    inserted_at_ms: timestamp_ms(),
                };
                if let Err(e) = self.inner.store.save_active_call(state).await {
                    warn!(error = %e, "PersistCallState failed");
                }
            }
            Effect::DeletePersistedCall { call_id } => {
                if let Err(e) = self
                    .inner
                    .store
                    .delete_active_call(&self.inner.owner_key, &call_id)
                    .await
                {
                    warn!(error = %e, "DeletePersistedCall failed");
                }
                // Drop any pending outbound envelopes for this call_id
                // so retries don't fire against a dead call.
                let _ = self.inner.queue.cancel_by_correlation(&call_id).await;
            }
            Effect::PersistMissedCall { .. } => {
                // missed_calls persistence is shell-specific (Tauri
                // uses a SQLite table; CLI/daemon may log to stdout
                // or skip). Default: emit a debug log here; the
                // upcoming `EnvelopeStore` extension (W16 follow-up)
                // adds a typed `record_missed_call` method.
                debug!("PersistMissedCall: not yet wired to EnvelopeStore (W16 follow-up)");
            }
            Effect::Notify(notif) => {
                self.inner.notifications.notify(&notif);
            }
        }
    }

    async fn queue_call_envelope(
        &self,
        recipient: &str,
        payload: &DmPayload,
        kind: EnvelopeKind,
        correlation_id: &str,
    ) {
        let bytes = match serialize_dm(payload) {
            Ok(b) => b,
            Err(e) => {
                warn!(error = %e, "queue_call_envelope: serialize_dm failed");
                return;
            }
        };
        if let Err(e) = self
            .inner
            .queue
            .send(recipient, bytes, kind, Some(correlation_id))
            .await
        {
            warn!(error = %e, "queue_call_envelope: queue send failed");
        }
    }

    /// W16.5b — dispatch a CallInvite via Veilid `app_call` and feed
    /// the result back into the state machine.
    ///
    /// On success, the receiver's `on_call` handler returns
    /// `CallResponse::CallRinging { call_id }`; we drive
    /// `CallEvent::RingingReceived` so the caller's UI flips
    /// "Calling…" → "Ringing…" on real evidence.
    ///
    /// On failure, we classify the transport error and drive
    /// `CallEvent::LocalUnreachable { reason }` so the state machine
    /// drops the Outgoing state, cancels the dialing timer, and emits
    /// `TransportNotification::CallUnreachable` for the UI.
    async fn dispatch_call_invite(
        &self,
        recipient: &str,
        call_id: &str,
        offer_kind: u8,
        initiator_x25519_pub: [u8; 32],
        expires_at_ms: u64,
    ) {
        use crate::frame::TypeId;
        use crate::payload::rpc::{CallInvitePayload, CallResponse};

        let invite = CallInvitePayload {
            call_id: call_id.to_string(),
            offer_kind,
            initiator_x25519_pub: initiator_x25519_pub.to_vec(),
            expires_at_ms,
        };
        let payload_bytes = match postcard::to_stdvec(&invite) {
            Ok(b) => b,
            Err(e) => {
                warn!(error = %e, "dispatch_call_invite: serialize failed");
                self.feed_unreachable(call_id, "send_failed").await;
                return;
            }
        };

        let result = self
            .inner
            .queue
            .send_app_call(
                recipient,
                TypeId::CallInvite,
                &payload_bytes,
                Duration::from_millis(CALL_INVITE_RPC_TIMEOUT_MS),
            )
            .await;

        match result {
            Ok(reply_bytes) => {
                // Parse the receiver's CallResponse::CallRinging.
                match postcard::from_bytes::<CallResponse>(&reply_bytes) {
                    Ok(CallResponse::CallRinging(ringing)) if ringing.call_id == call_id => {
                        let event = CallEvent::RingingReceived {
                            call_id: call_id.to_string(),
                        };
                        let effects = self.inner.state_machine.lock().apply(event);
                        self.interpret_effects(effects).await;
                    }
                    Ok(other) => {
                        warn!(
                            ?other,
                            call_id,
                            "dispatch_call_invite: unexpected CallResponse variant"
                        );
                        self.feed_unreachable(call_id, "send_failed").await;
                    }
                    Err(e) => {
                        warn!(error = %e, call_id, "dispatch_call_invite: reply parse failed");
                        self.feed_unreachable(call_id, "send_failed").await;
                    }
                }
            }
            Err(e) => {
                let reason = classify_call_invite_error(&e);
                debug!(error = %e, call_id, reason, "dispatch_call_invite: app_call failed");
                self.feed_unreachable(call_id, reason).await;
            }
        }
    }

    /// Helper: feed `CallEvent::LocalUnreachable` into the state machine
    /// and interpret resulting effects.
    async fn feed_unreachable(&self, call_id: &str, reason: &str) {
        let event = CallEvent::LocalUnreachable {
            call_id: call_id.to_string(),
            reason: reason.to_string(),
        };
        let effects = self.inner.state_machine.lock().apply(event);
        self.interpret_effects(effects).await;
    }

    /// W16.5b — receive-side handler for `InboundCall::CallInvite`.
    /// Drives the state machine with `CallEvent::InviteReceived`,
    /// interprets resulting effects (PersistCallState, SpawnIncomingTimer,
    /// Notify(IncomingCall)), and synthesizes the synchronous
    /// `CallResponse::CallRinging` reply.
    ///
    /// The caller (`InboundHandler::on_call`) returns this as the
    /// `app_call_reply` payload, which travels back to the sender as the
    /// `CallRinging` confirmation within Veilid's 5–10 s RPC budget.
    pub async fn handle_inbound_call_invite(
        &self,
        sender_pubkey: &str,
        sender_display_name: &str,
        invite: crate::payload::rpc::CallInvitePayload,
    ) -> crate::payload::rpc::CallResponse {
        use crate::payload::rpc::{CallResponse, CallRingingPayload};

        let kind = match invite.offer_kind {
            0 => CallKind::Audio,
            1 => CallKind::Video,
            other => {
                warn!(other, "handle_inbound_call_invite: invalid offer_kind");
                return CallResponse::Rejected {
                    reason: format!("invalid offer_kind: {other}"),
                };
            }
        };
        if invite.initiator_x25519_pub.len() != 32 {
            warn!(
                len = invite.initiator_x25519_pub.len(),
                "handle_inbound_call_invite: bad x25519 pub length"
            );
            return CallResponse::Rejected {
                reason: "bad initiator_x25519_pub length".into(),
            };
        }
        let mut peer_pub = [0u8; 32];
        peer_pub.copy_from_slice(&invite.initiator_x25519_pub);

        let now = timestamp_ms();
        let event = CallEvent::InviteReceived {
            call_id: invite.call_id.clone(),
            from: sender_pubkey.to_string(),
            from_display_name: sender_display_name.to_string(),
            kind,
            peer_x25519_pub: peer_pub,
            expires_at_ms: invite.expires_at_ms,
            received_at_ms: now,
        };
        let effects = self.inner.state_machine.lock().apply(event);
        if effects.is_empty() {
            // Duplicate invite for an existing call_id, etc. The state
            // machine is idempotent — return the same CallRinging reply
            // so the caller's perception is consistent.
            debug!(call_id = invite.call_id, "duplicate CallInvite — replying CallRinging");
            return CallResponse::CallRinging(CallRingingPayload {
                call_id: invite.call_id,
            });
        }
        self.interpret_effects(effects).await;
        CallResponse::CallRinging(CallRingingPayload {
            call_id: invite.call_id,
        })
    }

    fn spawn_timeout(&self, call_id: String, expires_at_ms: u64, kind: TimerKind) {
        let now = timestamp_ms();
        let remaining_ms = expires_at_ms.saturating_sub(now).max(1);
        let runtime = self.clone();
        let cid = call_id.clone();
        let handle = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(remaining_ms)).await;
            // Drive state machine with the timeout event. The state
            // machine no-ops if the call has already left
            // Outgoing/Incoming.
            let event = match kind {
                TimerKind::Dialing => CallEvent::LocalDialingTimeout { call_id: cid },
                TimerKind::Incoming => CallEvent::LocalIncomingTimeout { call_id: cid },
            };
            let effects = runtime.inner.state_machine.lock().apply(event);
            runtime.interpret_effects(effects).await;
        });
        // Cancel any prior timer for this call_id (e.g. re-spawn after
        // a state change). Prior handle's task aborts when dropped.
        self.inner.timers.lock().insert(call_id, handle);
    }
}

#[derive(Debug, Clone, Copy)]
enum TimerKind {
    Dialing,
    Incoming,
}

/// Boxed future returned by [`CallRuntime::interpret_effects`] so the
/// recursive call (StartVoiceSession failure → VoiceTransportDown
/// effects) can be awaited without blowing the stack via async fn
/// monomorphization.
type EffectsFuture =
    std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>;

fn generate_call_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// W16.5b — classify a transport error from the `app_call` CallInvite
/// dispatch into the four `LocalUnreachable` reasons that drive the
/// `CallUnreachable` notification.
fn classify_call_invite_error(e: &crate::error::TransportError) -> &'static str {
    use crate::error::TransportError;
    match e {
        TransportError::Timeout { .. } => "timeout",
        TransportError::NoRoute { .. } => "no_route",
        TransportError::SendFailed { reason, .. } => {
            // Inspect the underlying Veilid error string. Veilid maps
            // InvalidTarget / NoConnection through here.
            if reason.contains("InvalidTarget") || reason.contains("NoConnection") {
                "no_route"
            } else if reason.to_ascii_lowercase().contains("service") {
                "service_unavailable"
            } else {
                "send_failed"
            }
        }
        _ => "send_failed",
    }
}

fn kind_str(k: CallKind) -> &'static str {
    match k {
        CallKind::Audio => "audio",
        CallKind::Video => "video",
    }
}

fn status_str(s: CallStatus) -> &'static str {
    match s {
        CallStatus::Outgoing => "outgoing",
        CallStatus::Incoming => "incoming",
        CallStatus::Connecting => "connecting",
        CallStatus::Active => "active",
        CallStatus::Missed => "missed",
    }
}

/// Inverse of [`kind_str`]. Returns `None` for unrecognized strings.
fn parse_kind(s: &str) -> Option<CallKind> {
    match s {
        "audio" => Some(CallKind::Audio),
        "video" => Some(CallKind::Video),
        _ => None,
    }
}

/// Inverse of [`status_str`]. Returns `None` for unrecognized strings.
fn parse_status(s: &str) -> Option<CallStatus> {
    match s {
        "outgoing" => Some(CallStatus::Outgoing),
        "incoming" => Some(CallStatus::Incoming),
        "connecting" => Some(CallStatus::Connecting),
        "active" => Some(CallStatus::Active),
        "missed" => Some(CallStatus::Missed),
        _ => None,
    }
}
