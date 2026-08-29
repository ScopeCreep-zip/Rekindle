//! W16.5 — Pure-logic call state machine.
//!
//! Owns the lifecycle for 1:1 calls: maintains a `HashMap<CallId, CallState>`
//! and translates [`CallEvent`]s into [`Effect`]s. No async, no I/O, no
//! Veilid — the [`crate::Effect`] consumer (W16.7's `CallRuntime` in
//! rekindle-transport) interprets effects: sends envelopes via
//! [`EnvelopeQueue`], spawns ring timers, starts/stops voice sessions,
//! emits notifications.
//!
//! # Why a separate "pure logic" layer
//!
//! - **Testability**: state transitions test without spinning Veilid.
//! - **Determinism**: every (event, current state) → (next state, effects)
//!   tuple is reproducible, which makes property-based tests trivial.
//! - **Cross-frontend parity**: Tauri's `CallRuntime` and rekindle-cli's
//!   `CallRuntime` interpret the same effects against the same state
//!   machine; behavioral parity is structural, not by convention.
//!
//! # State diagram (1:1 calls)
//!
//! ```text
//!     LocalStartCall
//!         │
//!         ▼
//!      Outgoing ──AcceptReceived──▶ Connecting ──VoiceTransportUp──▶ Active
//!         │                              │                              │
//!         ├─DeclineReceived─▶ (drop)     │                              │
//!         ├─LocalCancel─▶ (drop)         │                              │
//!         ├─LocalDialingTimeout─▶ Missed │                              │
//!         └─EndReceived─▶ (drop)         │                              │
//!                                        ├─VoiceTransportDown─▶ (drop)  │
//!                                        └─EndReceived─▶ (drop)         │
//!                                                                       │
//!     InviteReceived                                                    │
//!         │                                                             │
//!         ▼                                                             │
//!      Incoming ──LocalAccept──▶ Connecting ────────────────────────────┘
//!         │
//!         ├─LocalDecline─▶ (drop)
//!         ├─EndReceived─▶ (drop)
//!         └─LocalIncomingTimeout─▶ Missed
//! ```

use std::collections::HashMap;

use rekindle_types::notification::TransportNotification;
use x25519_dalek::StaticSecret;
use zeroize::Zeroize;

use crate::state::{CallKind, CallState, CallStatus};

/// Inputs to [`CallStateMachine::apply`]. Each variant represents a
/// single observed change: a local user action, an inbound envelope, a
/// timer firing, or a voice-transport callback.
///
/// `StaticSecret` cannot derive `Clone` cleanly (it implements
/// `Clone` via `From`/`Into` only), so events that carry one are
/// constructed once and consumed by `apply`.
pub enum CallEvent {
    // ── Caller-side ─────────────────────────────────────────────────
    /// User clicked Voice/Video Call. `my_x25519_secret` is the
    /// freshly-generated keypair the caller will use for ECDH; the
    /// matching `my_x25519_pub` ships in the CallInvite envelope.
    LocalStartCall {
        call_id: String,
        peer: String,
        peer_display_name: String,
        kind: CallKind,
        my_x25519_secret: StaticSecret,
        my_x25519_pub: [u8; 32],
        expires_at_ms: u64,
        started_at_ms: u64,
    },

    /// User clicked Cancel on the OutgoingCallPanel.
    LocalCancel { call_id: String, reason: String },

    /// Caller's 30 s dialing timer fired without an accept.
    LocalDialingTimeout { call_id: String },

    /// W16.5b — Caller-side: the `app_call`-based CallInvite handshake
    /// failed within Veilid's 5–10 s RPC budget (Timeout, NoConnection,
    /// InvalidTarget, etc.). The peer is unreachable RIGHT NOW —
    /// distinct from `LocalDialingTimeout` which fires when the peer
    /// IS reachable but didn't answer within the 30 s ring window.
    LocalUnreachable {
        call_id: String,
        /// Classification: `"timeout"` | `"no_route"` |
        /// `"service_unavailable"` | `"send_failed"`.
        reason: String,
    },

    /// Inbound CallAccept (caller-side: peer accepted our outgoing).
    AcceptReceived {
        call_id: String,
        from: String,
        peer_x25519_pub: [u8; 32],
    },

    /// Inbound CallDecline (caller-side: peer rejected our outgoing).
    DeclineReceived { call_id: String, reason: String },

    /// Inbound CallRinging (caller-side: alerting ack — peer is
    /// ringing the user).
    RingingReceived { call_id: String },

    // ── Receiver-side ───────────────────────────────────────────────
    /// Inbound CallInvite. Receiver decides whether to accept/decline.
    InviteReceived {
        call_id: String,
        from: String,
        from_display_name: String,
        kind: CallKind,
        peer_x25519_pub: [u8; 32],
        expires_at_ms: u64,
        received_at_ms: u64,
    },

    /// User clicked Accept on the IncomingCallModal.
    LocalAccept {
        call_id: String,
        my_x25519_secret: StaticSecret,
        my_x25519_pub: [u8; 32],
    },

    /// User clicked Decline on the IncomingCallModal.
    LocalDecline { call_id: String, reason: String },

    /// Receiver's 30 s incoming timer fired without the user
    /// answering.
    LocalIncomingTimeout { call_id: String },

    // ── Either side ─────────────────────────────────────────────────
    /// Inbound CallEnd. Works in any state (Outgoing/Incoming/
    /// Connecting/Active) so cancel-while-ringing or
    /// hangup-mid-call cleans up cleanly.
    EndReceived { call_id: String, reason: String },

    /// Voice transport finished bringing up audio + signalling key
    /// + jitter buffer. Transitions Connecting → Active.
    VoiceTransportUp { call_id: String },

    /// Voice transport failed or dropped after Active. Treated as a
    /// hangup with a reason.
    VoiceTransportDown { call_id: String, reason: String },
}

/// Caller-side call-setup field set for [`CallEvent::LocalStartCall`].
/// Grouped into one owned struct so the `apply` helper takes a single
/// argument; `StaticSecret` is move-only, so the fields are consumed.
struct StartCallParams {
    call_id: String,
    peer: String,
    peer_display_name: String,
    kind: CallKind,
    my_x25519_secret: StaticSecret,
    my_x25519_pub: [u8; 32],
    expires_at_ms: u64,
    started_at_ms: u64,
}

/// Receiver-side invite field set for [`CallEvent::InviteReceived`].
/// Grouped into one owned struct so the `apply` helper takes a single
/// argument.
struct InviteReceivedParams {
    call_id: String,
    from: String,
    from_display_name: String,
    kind: CallKind,
    peer_x25519_pub: [u8; 32],
    expires_at_ms: u64,
    received_at_ms: u64,
}

/// Side-effects produced by [`CallStateMachine::apply`]. The runtime
/// (W16.7) interprets these against transport, voice subsystem, store,
/// timers, and the [`SharedState`] notification channel.
#[derive(Debug, Clone)]
pub enum Effect {
    // ── Envelope sends ──────────────────────────────────────────────
    //
    // The runtime serializes the matching `DmPayload` variant and
    // calls `EnvelopeQueue::send` with the right `EnvelopeKind`.
    SendCallInvite {
        recipient: String,
        call_id: String,
        offer_kind: u8,
        initiator_x25519_pub: [u8; 32],
        expires_at_ms: u64,
    },
    SendCallAccept {
        recipient: String,
        call_id: String,
        acceptor_x25519_pub: [u8; 32],
    },
    SendCallDecline {
        recipient: String,
        call_id: String,
        reason: String,
    },
    SendCallEnd {
        recipient: String,
        call_id: String,
        reason: String,
    },
    // (Effect::SendCallRinging dropped per W16.5b — the CallRinging
    //  reply is synthesized synchronously by the runtime's `on_call`
    //  handler as `CallResponse::CallRinging` inside `app_call_reply`,
    //  not produced as an outbound envelope.)

    // ── Voice session ───────────────────────────────────────────────
    /// Bring up audio + jitter buffer for this call. The `call_key` is
    /// the X25519-ECDH-derived shared secret, used by the voice
    /// transport for AEAD on every frame (W13.14).
    StartVoiceSession {
        call_id: String,
        peer: String,
        kind: CallKind,
        call_key: [u8; 32],
    },

    /// Tear down the voice session for this call.
    StopVoiceSession { call_id: String, reason: String },

    // ── Ring timers ─────────────────────────────────────────────────
    /// Spawn a 30 s dialing-side timeout. On fire, the timer task
    /// invokes `apply(LocalDialingTimeout)`.
    SpawnDialingTimer { call_id: String, expires_at_ms: u64 },

    /// Spawn a 30 s incoming-side timeout.
    SpawnIncomingTimer { call_id: String, expires_at_ms: u64 },

    /// Cancel any spawned timer for this call (call accepted, declined,
    /// ended, etc.).
    CancelTimer { call_id: String },

    // ── Persistence ─────────────────────────────────────────────────
    /// Persist Outgoing/Incoming state for crash recovery (W16.8).
    /// Active state intentionally NOT persisted — voice transport
    /// can't meaningfully resume across crash.
    PersistCallState {
        call_id: String,
        peer_pubkey: String,
        kind: CallKind,
        status: CallStatus,
        expires_at_ms: u64,
        /// Outgoing-side: our X25519 secret bytes (for resume).
        my_x25519_secret: Option<[u8; 32]>,
        /// Incoming-side: peer's X25519 pub (we have it; they won't
        /// re-send if we restart and re-emit IncomingCall).
        peer_x25519_pub: Option<[u8; 32]>,
    },

    /// Delete persisted call state (call ended, declined, etc.).
    DeletePersistedCall { call_id: String },

    /// Persist a missed_calls row (timeout fired, no accept).
    PersistMissedCall {
        call_id: String,
        peer_pubkey: String,
        kind: CallKind,
        expired_at_ms: u64,
    },

    // ── Notifications ───────────────────────────────────────────────
    /// Emit a [`TransportNotification`] via [`SharedState::notify`].
    /// Each frontend (Tauri / CLI / daemon) bridges to its own surface.
    Notify(TransportNotification),
}

/// Pure-logic state machine for 1:1 calls.
///
/// Cloned cheaply (the only state is a `HashMap<String, CallState>`).
/// The runtime owns one instance, mutates via `apply`, and interprets
/// the returned effects.
pub struct CallStateMachine {
    active: HashMap<String, CallState>,
}

impl Default for CallStateMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl CallStateMachine {
    #[must_use]
    pub fn new() -> Self {
        Self {
            active: HashMap::new(),
        }
    }

    /// Snapshot of all active call states. For introspection/UI.
    pub fn snapshot(&self) -> Vec<&CallState> {
        self.active.values().collect()
    }

    /// Look up a single call by id.
    #[must_use]
    pub fn get(&self, call_id: &str) -> Option<&CallState> {
        self.active.get(call_id)
    }

    /// True if the local user has an outgoing call to a given peer
    /// (used by the receiver-side dispatch to detect glare).
    #[must_use]
    pub fn has_outgoing_to(&self, peer: &str) -> bool {
        self.active
            .values()
            .any(|c| c.peer_pubkey == peer && matches!(c.status, CallStatus::Outgoing))
    }

    /// W16.8 — rehydrate a `CallState` from persistence (crash
    /// recovery). Direct insert; no `Effect`s. The runtime is
    /// responsible for separately spawning the matching ring timer
    /// and emitting the matching notification.
    ///
    /// Only called from the recovery path. Replaces any existing entry
    /// with the same call_id (typical use: rehydrate before any new
    /// events arrive, so collisions don't happen in practice).
    pub fn rehydrate(&mut self, state: CallState) {
        self.active.insert(state.call_id.clone(), state);
    }

    /// Drive a single event. Returns the side-effects the runtime
    /// should perform. Effects are returned in the order they should
    /// fire.
    pub fn apply(&mut self, event: CallEvent) -> Vec<Effect> {
        match event {
            CallEvent::LocalStartCall {
                call_id,
                peer,
                peer_display_name,
                kind,
                my_x25519_secret,
                my_x25519_pub,
                expires_at_ms,
                started_at_ms,
            } => self.apply_local_start_call(StartCallParams {
                call_id,
                peer,
                peer_display_name,
                kind,
                my_x25519_secret,
                my_x25519_pub,
                expires_at_ms,
                started_at_ms,
            }),
            CallEvent::LocalCancel { call_id, reason } => self.apply_local_cancel(&call_id, reason),
            CallEvent::LocalDialingTimeout { call_id } => {
                self.apply_local_dialing_timeout(&call_id)
            }
            CallEvent::LocalUnreachable { call_id, reason } => {
                self.apply_local_unreachable(&call_id, reason)
            }
            CallEvent::AcceptReceived {
                call_id,
                from,
                peer_x25519_pub,
            } => self.apply_accept_received(&call_id, &from, peer_x25519_pub),
            CallEvent::DeclineReceived { call_id, reason } => {
                self.apply_decline_received(&call_id, reason)
            }
            CallEvent::RingingReceived { call_id } => self.apply_ringing_received(&call_id),
            CallEvent::InviteReceived {
                call_id,
                from,
                from_display_name,
                kind,
                peer_x25519_pub,
                expires_at_ms,
                received_at_ms,
            } => self.apply_invite_received(InviteReceivedParams {
                call_id,
                from,
                from_display_name,
                kind,
                peer_x25519_pub,
                expires_at_ms,
                received_at_ms,
            }),
            CallEvent::LocalAccept {
                call_id,
                my_x25519_secret,
                my_x25519_pub,
            } => self.apply_local_accept(&call_id, my_x25519_secret, my_x25519_pub),
            CallEvent::LocalDecline { call_id, reason } => {
                self.apply_local_decline(&call_id, reason)
            }
            CallEvent::LocalIncomingTimeout { call_id } => {
                self.apply_local_incoming_timeout(&call_id)
            }
            CallEvent::EndReceived { call_id, reason } => self.apply_end_received(&call_id, reason),
            CallEvent::VoiceTransportUp { call_id } => self.apply_voice_transport_up(&call_id),
            CallEvent::VoiceTransportDown { call_id, reason } => {
                self.apply_voice_transport_down(&call_id, reason)
            }
        }
    }
}

mod incoming;
mod media;
mod outgoing;

/// Wire-string for [`CallKind`] used in [`TransportNotification`]
/// payloads. "audio" / "video" matches the existing chat-event schema.
fn kind_str(k: CallKind) -> &'static str {
    match k {
        CallKind::Audio => "audio",
        CallKind::Video => "video",
    }
}

/// Drop impl shared with [`CallState`]: zero out any in-flight key
/// material when the state machine is dropped.
impl Drop for CallStateMachine {
    fn drop(&mut self) {
        for state in self.active.values_mut() {
            if let Some(ref mut k) = state.call_key {
                k.zeroize();
            }
        }
    }
}

#[cfg(test)]
mod tests;
