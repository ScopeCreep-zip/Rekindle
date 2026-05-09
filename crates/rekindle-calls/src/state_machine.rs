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
    LocalCancel {
        call_id: String,
        reason: String,
    },

    /// Caller's 30 s dialing timer fired without an accept.
    LocalDialingTimeout {
        call_id: String,
    },

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
    DeclineReceived {
        call_id: String,
        reason: String,
    },

    /// Inbound CallRinging (caller-side: alerting ack — peer is
    /// ringing the user).
    RingingReceived {
        call_id: String,
    },

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
    LocalDecline {
        call_id: String,
        reason: String,
    },

    /// Receiver's 30 s incoming timer fired without the user
    /// answering.
    LocalIncomingTimeout {
        call_id: String,
    },

    // ── Either side ─────────────────────────────────────────────────

    /// Inbound CallEnd. Works in any state (Outgoing/Incoming/
    /// Connecting/Active) so cancel-while-ringing or
    /// hangup-mid-call cleans up cleanly.
    EndReceived {
        call_id: String,
        reason: String,
    },

    /// Voice transport finished bringing up audio + signalling key
    /// + jitter buffer. Transitions Connecting → Active.
    VoiceTransportUp {
        call_id: String,
    },

    /// Voice transport failed or dropped after Active. Treated as a
    /// hangup with a reason.
    VoiceTransportDown {
        call_id: String,
        reason: String,
    },
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
    StopVoiceSession {
        call_id: String,
        reason: String,
    },

    // ── Ring timers ─────────────────────────────────────────────────

    /// Spawn a 30 s dialing-side timeout. On fire, the timer task
    /// invokes `apply(LocalDialingTimeout)`.
    SpawnDialingTimer {
        call_id: String,
        expires_at_ms: u64,
    },

    /// Spawn a 30 s incoming-side timeout.
    SpawnIncomingTimer {
        call_id: String,
        expires_at_ms: u64,
    },

    /// Cancel any spawned timer for this call (call accepted, declined,
    /// ended, etc.).
    CancelTimer {
        call_id: String,
    },

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
    DeletePersistedCall {
        call_id: String,
    },

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
            } => self.apply_local_start_call(
                &call_id,
                &peer,
                peer_display_name,
                kind,
                my_x25519_secret,
                my_x25519_pub,
                expires_at_ms,
                started_at_ms,
            ),
            CallEvent::LocalCancel { call_id, reason } => self.apply_local_cancel(&call_id, reason),
            CallEvent::LocalDialingTimeout { call_id } => self.apply_local_dialing_timeout(&call_id),
            CallEvent::LocalUnreachable { call_id, reason } => self.apply_local_unreachable(&call_id, reason),
            CallEvent::AcceptReceived {
                call_id,
                from,
                peer_x25519_pub,
            } => self.apply_accept_received(&call_id, &from, peer_x25519_pub),
            CallEvent::DeclineReceived { call_id, reason } => self.apply_decline_received(&call_id, reason),
            CallEvent::RingingReceived { call_id } => self.apply_ringing_received(&call_id),
            CallEvent::InviteReceived {
                call_id,
                from,
                from_display_name,
                kind,
                peer_x25519_pub,
                expires_at_ms,
                received_at_ms,
            } => self.apply_invite_received(
                call_id,
                from,
                from_display_name,
                kind,
                peer_x25519_pub,
                expires_at_ms,
                received_at_ms,
            ),
            CallEvent::LocalAccept {
                call_id,
                my_x25519_secret,
                my_x25519_pub,
            } => self.apply_local_accept(&call_id, my_x25519_secret, my_x25519_pub),
            CallEvent::LocalDecline { call_id, reason } => self.apply_local_decline(&call_id, reason),
            CallEvent::LocalIncomingTimeout { call_id } => self.apply_local_incoming_timeout(&call_id),
            CallEvent::EndReceived { call_id, reason } => self.apply_end_received(&call_id, reason),
            CallEvent::VoiceTransportUp { call_id } => self.apply_voice_transport_up(&call_id),
            CallEvent::VoiceTransportDown { call_id, reason } => {
                self.apply_voice_transport_down(&call_id, reason)
            }
        }
    }

    // ── apply_* helpers ─────────────────────────────────────────────

    #[allow(clippy::too_many_arguments)]
    fn apply_local_start_call(
        &mut self,
        call_id: &str,
        peer: &str,
        peer_display_name: String,
        kind: CallKind,
        my_x25519_secret: StaticSecret,
        my_x25519_pub: [u8; 32],
        expires_at_ms: u64,
        started_at_ms: u64,
    ) -> Vec<Effect> {
        let state = CallState {
            call_id: call_id.into(),
            peer_pubkey: peer.into(),
            kind,
            status: CallStatus::Outgoing,
            expires_at_ms,
            my_x25519_secret: Some(my_x25519_secret),
            peer_x25519_pub: None,
            call_key: None,
        };
        let secret_bytes = state.my_x25519_secret.as_ref().map(StaticSecret::to_bytes);
        self.active.insert(call_id.into(), state);

        vec![
            Effect::PersistCallState {
                call_id: call_id.into(),
                peer_pubkey: peer.into(),
                kind,
                status: CallStatus::Outgoing,
                expires_at_ms,
                my_x25519_secret: secret_bytes,
                peer_x25519_pub: None,
            },
            Effect::SendCallInvite {
                recipient: peer.into(),
                call_id: call_id.into(),
                offer_kind: kind.as_u8(),
                initiator_x25519_pub: my_x25519_pub,
                expires_at_ms,
            },
            Effect::SpawnDialingTimer {
                call_id: call_id.into(),
                expires_at_ms,
            },
            Effect::Notify(TransportNotification::CallStarted {
                call_id: call_id.into(),
                kind: kind_str(kind).into(),
                peer_key: peer.into(),
                peer_display_name,
                expires_at_ms,
                started_at_ms,
                status: "calling".into(),
            }),
            Effect::Notify(TransportNotification::ConversationFocusRequested {
                peer_key: peer.into(),
                display_name: String::new(),
                reason: "call-started".into(),
            }),
        ]
    }

    fn apply_local_cancel(&mut self, call_id: &str, reason: String) -> Vec<Effect> {
        let Some(state) = self.active.remove(call_id) else {
            return vec![];
        };
        let mut effects = vec![
            Effect::CancelTimer {
                call_id: call_id.into(),
            },
            Effect::SendCallEnd {
                recipient: state.peer_pubkey.clone(),
                call_id: call_id.into(),
                reason: reason.clone(),
            },
            Effect::DeletePersistedCall {
                call_id: call_id.into(),
            },
        ];
        if matches!(state.status, CallStatus::Active | CallStatus::Connecting) {
            effects.push(Effect::StopVoiceSession {
                call_id: call_id.into(),
                reason: reason.clone(),
            });
        }
        effects.push(Effect::Notify(TransportNotification::CallEnded {
            call_id: call_id.into(),
            reason,
        }));
        effects
    }

    fn apply_local_dialing_timeout(&mut self, call_id: &str) -> Vec<Effect> {
        // Only fire if the call is STILL Outgoing — accept/decline race
        // could have already removed/transitioned it.
        let still_outgoing = self
            .active
            .get(call_id)
            .is_some_and(|c| matches!(c.status, CallStatus::Outgoing));
        if !still_outgoing {
            return vec![];
        }
        let Some(state) = self.active.remove(call_id) else {
            return vec![];
        };
        vec![
            Effect::DeletePersistedCall {
                call_id: call_id.into(),
            },
            Effect::PersistMissedCall {
                call_id: call_id.into(),
                peer_pubkey: state.peer_pubkey.clone(),
                kind: state.kind,
                expired_at_ms: state.expires_at_ms,
            },
            Effect::Notify(TransportNotification::CallTimedOut {
                call_id: call_id.into(),
            }),
        ]
    }

    /// W16.5b — `app_call` CallInvite failed inside Veilid's RPC budget
    /// (peer offline, no route, etc.). Drops Outgoing state and emits
    /// `CallUnreachable` so the caller's UI can surface "Couldn't reach
    /// {peer}" within ~10 s instead of waiting for the 30 s ring timer.
    /// No missed_call row — the receiver never saw the invite.
    fn apply_local_unreachable(&mut self, call_id: &str, reason: String) -> Vec<Effect> {
        let still_outgoing = self
            .active
            .get(call_id)
            .is_some_and(|c| matches!(c.status, CallStatus::Outgoing));
        if !still_outgoing {
            // Race: accept arrived before the app_call returned its
            // error path; treat the call as alive and ignore the
            // unreachable signal. (Should be rare given app_call's
            // 5–10 s budget vs the user-decision time.)
            return vec![];
        }
        let _ = self.active.remove(call_id);
        vec![
            Effect::CancelTimer {
                call_id: call_id.into(),
            },
            Effect::DeletePersistedCall {
                call_id: call_id.into(),
            },
            Effect::Notify(TransportNotification::CallUnreachable {
                call_id: call_id.into(),
                reason,
            }),
        ]
    }

    fn apply_accept_received(
        &mut self,
        call_id: &str,
        from: &str,
        peer_x25519_pub: [u8; 32],
    ) -> Vec<Effect> {
        let Some(state) = self.active.get_mut(call_id) else {
            return vec![];
        };
        // Validate: status must be Outgoing AND sender must match peer.
        if !matches!(state.status, CallStatus::Outgoing) || state.peer_pubkey != from {
            return vec![];
        }

        // Derive call_key. If derivation fails, we can't proceed —
        // tear down with a clear reason.
        let Some(my_secret) = state.my_x25519_secret.as_ref() else {
            // Defensive: we should always have our secret on Outgoing.
            // If not, abort.
            let cid = call_id.to_string();
            let peer = state.peer_pubkey.clone();
            self.active.remove(call_id);
            return vec![
                Effect::CancelTimer { call_id: cid.clone() },
                Effect::SendCallEnd {
                    recipient: peer,
                    call_id: cid.clone(),
                    reason: "missing local x25519 secret".into(),
                },
                Effect::DeletePersistedCall { call_id: cid.clone() },
                Effect::Notify(TransportNotification::CallEnded {
                    call_id: cid,
                    reason: "internal: missing x25519 secret".into(),
                }),
            ];
        };
        let call_key = match crate::derive_call_key(my_secret, &peer_x25519_pub, call_id) {
            Ok(k) => k,
            Err(e) => {
                let cid = call_id.to_string();
                let peer = state.peer_pubkey.clone();
                self.active.remove(call_id);
                return vec![
                    Effect::CancelTimer { call_id: cid.clone() },
                    Effect::SendCallEnd {
                        recipient: peer,
                        call_id: cid.clone(),
                        reason: format!("call_key derive failed: {e}"),
                    },
                    Effect::DeletePersistedCall { call_id: cid.clone() },
                    Effect::Notify(TransportNotification::CallEnded {
                        call_id: cid,
                        reason: format!("call_key derive failed: {e}"),
                    }),
                ];
            }
        };

        state.peer_x25519_pub = Some(peer_x25519_pub);
        state.call_key = Some(call_key);
        state.status = CallStatus::Connecting;
        let kind = state.kind;
        let peer = state.peer_pubkey.clone();
        let now = rekindle_utils::timestamp_ms();

        vec![
            Effect::CancelTimer {
                call_id: call_id.into(),
            },
            Effect::StartVoiceSession {
                call_id: call_id.into(),
                peer: peer.clone(),
                kind,
                call_key,
            },
            Effect::Notify(TransportNotification::CallStatusChanged {
                call_id: call_id.into(),
                status: "connecting".into(),
                timestamp_ms: now,
            }),
            // CallConnected fires on VoiceTransportUp once audio is live.
            Effect::Notify(TransportNotification::ConversationFocusRequested {
                peer_key: peer,
                display_name: String::new(),
                reason: "call-accepted".into(),
            }),
        ]
    }

    fn apply_decline_received(&mut self, call_id: &str, reason: String) -> Vec<Effect> {
        let Some(state) = self.active.remove(call_id) else {
            return vec![];
        };
        // Only valid if the call was Outgoing (caller-side).
        if !matches!(state.status, CallStatus::Outgoing) {
            // Re-insert (unexpected state); ignore.
            self.active.insert(call_id.to_string(), state);
            return vec![];
        }
        vec![
            Effect::CancelTimer {
                call_id: call_id.into(),
            },
            Effect::DeletePersistedCall {
                call_id: call_id.into(),
            },
            Effect::Notify(TransportNotification::CallDeclined {
                call_id: call_id.into(),
                reason,
            }),
        ]
    }

    fn apply_ringing_received(&self, call_id: &str) -> Vec<Effect> {
        // Pure UI hint: caller's panel transitions "Calling…" → "Ringing…".
        // Only valid for Outgoing calls.
        let still_outgoing = self
            .active
            .get(call_id)
            .is_some_and(|c| matches!(c.status, CallStatus::Outgoing));
        if !still_outgoing {
            return vec![];
        }
        let now = rekindle_utils::timestamp_ms();
        vec![
            Effect::Notify(TransportNotification::CallRinging {
                call_id: call_id.into(),
            }),
            Effect::Notify(TransportNotification::CallStatusChanged {
                call_id: call_id.into(),
                status: "ringing".into(),
                timestamp_ms: now,
            }),
        ]
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_invite_received(
        &mut self,
        call_id: String,
        from: String,
        from_display_name: String,
        kind: CallKind,
        peer_x25519_pub: [u8; 32],
        expires_at_ms: u64,
        received_at_ms: u64,
    ) -> Vec<Effect> {
        // Reject duplicate invite for an existing call_id.
        if self.active.contains_key(&call_id) {
            return vec![];
        }

        let state = CallState {
            call_id: call_id.clone(),
            peer_pubkey: from.clone(),
            kind,
            status: CallStatus::Incoming,
            expires_at_ms,
            my_x25519_secret: None,
            peer_x25519_pub: Some(peer_x25519_pub),
            call_key: None,
        };
        self.active.insert(call_id.clone(), state);

        // W16.5b — note the absence of an `Effect::SendCallRinging`:
        // the runtime's `on_call` handler synthesizes
        // `CallResponse::CallRinging { call_id }` and returns it as
        // `app_call_reply` synchronously. No outbound envelope.
        vec![
            Effect::PersistCallState {
                call_id: call_id.clone(),
                peer_pubkey: from.clone(),
                kind,
                status: CallStatus::Incoming,
                expires_at_ms,
                my_x25519_secret: None,
                peer_x25519_pub: Some(peer_x25519_pub),
            },
            Effect::SpawnIncomingTimer {
                call_id: call_id.clone(),
                expires_at_ms,
            },
            Effect::Notify(TransportNotification::IncomingCall {
                call_id,
                kind: kind_str(kind).into(),
                from,
                display_name: from_display_name,
                expires_at_ms,
                received_at_ms,
                is_group: false,
            }),
        ]
    }

    fn apply_local_accept(
        &mut self,
        call_id: &str,
        my_x25519_secret: StaticSecret,
        _my_x25519_pub: [u8; 32],
    ) -> Vec<Effect> {
        let Some(state) = self.active.get_mut(call_id) else {
            return vec![];
        };
        if !matches!(state.status, CallStatus::Incoming) {
            return vec![];
        }
        // Receive-side already has peer_x25519_pub from the invite.
        let Some(peer_pub) = state.peer_x25519_pub else {
            // Defensive: shouldn't happen on Incoming.
            return vec![];
        };

        let call_key = match crate::derive_call_key(&my_x25519_secret, &peer_pub, call_id) {
            Ok(k) => k,
            Err(e) => {
                let cid = call_id.to_string();
                let peer = state.peer_pubkey.clone();
                self.active.remove(call_id);
                return vec![
                    Effect::CancelTimer { call_id: cid.clone() },
                    Effect::SendCallDecline {
                        recipient: peer,
                        call_id: cid.clone(),
                        reason: format!("call_key derive failed: {e}"),
                    },
                    Effect::DeletePersistedCall { call_id: cid.clone() },
                    Effect::Notify(TransportNotification::CallEnded {
                        call_id: cid,
                        reason: format!("call_key derive failed: {e}"),
                    }),
                ];
            }
        };

        // Compute the matching public key for the SendCallAccept effect.
        let my_pub_for_accept = x25519_dalek::PublicKey::from(&my_x25519_secret);
        state.my_x25519_secret = Some(my_x25519_secret);
        state.call_key = Some(call_key);
        state.status = CallStatus::Connecting;
        let kind = state.kind;
        let peer = state.peer_pubkey.clone();
        let now = rekindle_utils::timestamp_ms();

        vec![
            Effect::CancelTimer {
                call_id: call_id.into(),
            },
            Effect::StartVoiceSession {
                call_id: call_id.into(),
                peer: peer.clone(),
                kind,
                call_key,
            },
            Effect::SendCallAccept {
                recipient: peer.clone(),
                call_id: call_id.into(),
                acceptor_x25519_pub: my_pub_for_accept.to_bytes(),
            },
            Effect::Notify(TransportNotification::CallStatusChanged {
                call_id: call_id.into(),
                status: "connecting".into(),
                timestamp_ms: now,
            }),
            Effect::Notify(TransportNotification::ConversationFocusRequested {
                peer_key: peer,
                display_name: String::new(),
                reason: "call-accepted".into(),
            }),
        ]
    }

    fn apply_local_decline(&mut self, call_id: &str, reason: String) -> Vec<Effect> {
        let Some(state) = self.active.remove(call_id) else {
            return vec![];
        };
        if !matches!(state.status, CallStatus::Incoming) {
            // Wrong state — re-insert and ignore.
            self.active.insert(call_id.to_string(), state);
            return vec![];
        }
        vec![
            Effect::CancelTimer {
                call_id: call_id.into(),
            },
            Effect::SendCallDecline {
                recipient: state.peer_pubkey.clone(),
                call_id: call_id.into(),
                reason: reason.clone(),
            },
            Effect::DeletePersistedCall {
                call_id: call_id.into(),
            },
            // Receiver's own UI also clears the modal slot.
            Effect::Notify(TransportNotification::CallDeclined {
                call_id: call_id.into(),
                reason,
            }),
        ]
    }

    fn apply_local_incoming_timeout(&mut self, call_id: &str) -> Vec<Effect> {
        let still_incoming = self
            .active
            .get(call_id)
            .is_some_and(|c| matches!(c.status, CallStatus::Incoming));
        if !still_incoming {
            return vec![];
        }
        let Some(state) = self.active.remove(call_id) else {
            return vec![];
        };
        vec![
            Effect::DeletePersistedCall {
                call_id: call_id.into(),
            },
            Effect::PersistMissedCall {
                call_id: call_id.into(),
                peer_pubkey: state.peer_pubkey.clone(),
                kind: state.kind,
                expired_at_ms: state.expires_at_ms,
            },
            Effect::Notify(TransportNotification::CallMissed {
                call_id: call_id.into(),
                from: state.peer_pubkey.clone(),
            }),
        ]
    }

    fn apply_end_received(&mut self, call_id: &str, reason: String) -> Vec<Effect> {
        let Some(state) = self.active.remove(call_id) else {
            return vec![];
        };
        let mut effects = vec![
            Effect::CancelTimer {
                call_id: call_id.into(),
            },
            Effect::DeletePersistedCall {
                call_id: call_id.into(),
            },
        ];
        if matches!(state.status, CallStatus::Active | CallStatus::Connecting) {
            effects.push(Effect::StopVoiceSession {
                call_id: call_id.into(),
                reason: reason.clone(),
            });
        }
        effects.push(Effect::Notify(TransportNotification::CallEnded {
            call_id: call_id.into(),
            reason,
        }));
        effects
    }

    fn apply_voice_transport_up(&mut self, call_id: &str) -> Vec<Effect> {
        let Some(state) = self.active.get_mut(call_id) else {
            return vec![];
        };
        if !matches!(state.status, CallStatus::Connecting) {
            return vec![];
        }
        state.status = CallStatus::Active;
        let kind = state.kind;
        let peer = state.peer_pubkey.clone();
        let now = rekindle_utils::timestamp_ms();
        vec![
            Effect::Notify(TransportNotification::CallConnected {
                call_id: call_id.into(),
                kind: kind_str(kind).into(),
                peer_key: peer,
                peer_display_name: String::new(),
                started_at_ms: now,
                expected_local_camera: matches!(kind, CallKind::Video),
            }),
            Effect::Notify(TransportNotification::CallStatusChanged {
                call_id: call_id.into(),
                status: "active".into(),
                timestamp_ms: now,
            }),
        ]
    }

    fn apply_voice_transport_down(&mut self, call_id: &str, reason: String) -> Vec<Effect> {
        let Some(state) = self.active.remove(call_id) else {
            return vec![];
        };
        let mut effects = vec![
            Effect::CancelTimer {
                call_id: call_id.into(),
            },
            Effect::DeletePersistedCall {
                call_id: call_id.into(),
            },
        ];
        if matches!(state.status, CallStatus::Active | CallStatus::Connecting) {
            effects.push(Effect::StopVoiceSession {
                call_id: call_id.into(),
                reason: reason.clone(),
            });
            // Tell the peer we're hanging up from our side.
            effects.push(Effect::SendCallEnd {
                recipient: state.peer_pubkey.clone(),
                call_id: call_id.into(),
                reason: reason.clone(),
            });
        }
        effects.push(Effect::Notify(TransportNotification::CallEnded {
            call_id: call_id.into(),
            reason,
        }));
        effects
    }
}

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
mod tests {
    use super::*;
    use crate::fresh_keypair;

    fn fresh_event_local_start(call_id: &str, peer: &str, kind: CallKind) -> CallEvent {
        let (sk, pk) = fresh_keypair();
        CallEvent::LocalStartCall {
            call_id: call_id.into(),
            peer: peer.into(),
            peer_display_name: format!("name-of-{peer}"),
            kind,
            my_x25519_secret: sk,
            my_x25519_pub: pk,
            expires_at_ms: 30_000,
            started_at_ms: 0,
        }
    }

    fn fresh_event_invite_received(call_id: &str, from: &str, kind: CallKind) -> CallEvent {
        let (_, peer_pub) = fresh_keypair();
        CallEvent::InviteReceived {
            call_id: call_id.into(),
            from: from.into(),
            from_display_name: format!("name-of-{from}"),
            kind,
            peer_x25519_pub: peer_pub,
            expires_at_ms: 30_000,
            received_at_ms: 0,
        }
    }

    #[test]
    fn local_start_emits_invite_timer_persist_notify() {
        let mut sm = CallStateMachine::new();
        let effects = sm.apply(fresh_event_local_start("c1", "bob", CallKind::Audio));
        assert!(effects.iter().any(|e| matches!(e, Effect::SendCallInvite { .. })));
        assert!(effects.iter().any(|e| matches!(e, Effect::SpawnDialingTimer { .. })));
        assert!(effects.iter().any(|e| matches!(e, Effect::PersistCallState { status: CallStatus::Outgoing, .. })));
        assert!(matches!(sm.get("c1"), Some(s) if matches!(s.status, CallStatus::Outgoing)));
    }

    #[test]
    fn invite_received_emits_timer_persist_notify_no_outbound_ringing() {
        // W16.5b — receiver no longer emits an outbound CallRinging
        // envelope; the runtime synthesizes the ringing reply
        // synchronously inside `app_call_reply`.
        let mut sm = CallStateMachine::new();
        let effects = sm.apply(fresh_event_invite_received("c1", "alice", CallKind::Video));
        assert!(effects.iter().any(|e| matches!(e, Effect::SpawnIncomingTimer { .. })));
        assert!(effects.iter().any(|e| matches!(e, Effect::PersistCallState { status: CallStatus::Incoming, .. })));
        assert!(effects.iter().any(|e| matches!(e, Effect::Notify(TransportNotification::IncomingCall { .. }))));
        assert!(matches!(sm.get("c1"), Some(s) if matches!(s.status, CallStatus::Incoming)));
    }

    #[test]
    fn local_unreachable_drops_state_emits_notification() {
        // W16.5b — caller's app_call CallInvite failed inside Veilid's
        // RPC budget. Drops Outgoing, emits CallUnreachable, no
        // missed_call row (receiver never saw the invite).
        let mut sm = CallStateMachine::new();
        let _ = sm.apply(fresh_event_local_start("c1", "bob", CallKind::Audio));
        let effects = sm.apply(CallEvent::LocalUnreachable {
            call_id: "c1".into(),
            reason: "timeout".into(),
        });
        assert!(effects.iter().any(|e| matches!(e, Effect::CancelTimer { .. })));
        assert!(effects.iter().any(|e| matches!(e, Effect::DeletePersistedCall { .. })));
        assert!(effects.iter().any(|e| matches!(e,
            Effect::Notify(TransportNotification::CallUnreachable { reason, .. }) if reason == "timeout"
        )));
        // No PersistMissedCall — receiver never knew about the call.
        assert!(!effects.iter().any(|e| matches!(e, Effect::PersistMissedCall { .. })));
        assert!(sm.get("c1").is_none());
    }

    #[test]
    fn local_unreachable_after_accept_is_noop() {
        // Race: accept arrived before app_call returned its error path.
        // Treat the call as alive and ignore the unreachable signal.
        let mut sm = CallStateMachine::new();
        let _ = sm.apply(fresh_event_local_start("c1", "bob", CallKind::Audio));
        let (_, peer_pub) = fresh_keypair();
        let _ = sm.apply(CallEvent::AcceptReceived {
            call_id: "c1".into(),
            from: "bob".into(),
            peer_x25519_pub: peer_pub,
        });
        let effects = sm.apply(CallEvent::LocalUnreachable {
            call_id: "c1".into(),
            reason: "timeout".into(),
        });
        assert!(effects.is_empty(), "unreachable post-accept must be noop");
        assert!(sm.get("c1").is_some(), "call still alive after accept");
    }

    #[test]
    fn duplicate_invite_for_same_call_id_dropped() {
        let mut sm = CallStateMachine::new();
        let _ = sm.apply(fresh_event_invite_received("c1", "alice", CallKind::Audio));
        let effects = sm.apply(fresh_event_invite_received("c1", "alice", CallKind::Audio));
        assert!(effects.is_empty(), "duplicate invite produces no effects");
    }

    #[test]
    fn outgoing_then_decline_received_transitions_to_dropped() {
        let mut sm = CallStateMachine::new();
        let _ = sm.apply(fresh_event_local_start("c1", "bob", CallKind::Audio));
        let effects = sm.apply(CallEvent::DeclineReceived {
            call_id: "c1".into(),
            reason: "busy".into(),
        });
        assert!(effects.iter().any(|e| matches!(e, Effect::CancelTimer { .. })));
        assert!(effects.iter().any(|e| matches!(e, Effect::DeletePersistedCall { .. })));
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::Notify(TransportNotification::CallDeclined { .. })))
        );
        assert!(sm.get("c1").is_none(), "call removed after decline");
    }

    #[test]
    fn dialing_timeout_persists_missed_call() {
        let mut sm = CallStateMachine::new();
        let _ = sm.apply(fresh_event_local_start("c1", "bob", CallKind::Audio));
        let effects = sm.apply(CallEvent::LocalDialingTimeout {
            call_id: "c1".into(),
        });
        assert!(effects.iter().any(|e| matches!(e, Effect::PersistMissedCall { .. })));
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::Notify(TransportNotification::CallTimedOut { .. })))
        );
        assert!(sm.get("c1").is_none());
    }

    #[test]
    fn incoming_timeout_after_accept_does_nothing() {
        // Accept transitioned to Connecting → timer firing later is a
        // no-op (still_incoming check).
        let mut sm = CallStateMachine::new();
        let _ = sm.apply(fresh_event_invite_received("c1", "alice", CallKind::Audio));
        let (sk, pk) = fresh_keypair();
        let _ = sm.apply(CallEvent::LocalAccept {
            call_id: "c1".into(),
            my_x25519_secret: sk,
            my_x25519_pub: pk,
        });
        let effects = sm.apply(CallEvent::LocalIncomingTimeout {
            call_id: "c1".into(),
        });
        assert!(effects.is_empty(), "timeout after accept is a no-op");
    }

    #[test]
    fn end_received_drops_call_in_any_state() {
        // Outgoing case
        let mut sm = CallStateMachine::new();
        let _ = sm.apply(fresh_event_local_start("c1", "bob", CallKind::Audio));
        let effects = sm.apply(CallEvent::EndReceived {
            call_id: "c1".into(),
            reason: "cancelled".into(),
        });
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::Notify(TransportNotification::CallEnded { .. })))
        );
        assert!(sm.get("c1").is_none());

        // Incoming case
        let mut sm2 = CallStateMachine::new();
        let _ = sm2.apply(fresh_event_invite_received("c2", "alice", CallKind::Audio));
        let effects = sm2.apply(CallEvent::EndReceived {
            call_id: "c2".into(),
            reason: "caller hung up".into(),
        });
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::Notify(TransportNotification::CallEnded { .. })))
        );
        assert!(sm2.get("c2").is_none());
    }

    #[test]
    fn voice_transport_up_transitions_connecting_to_active() {
        let mut sm = CallStateMachine::new();
        let _ = sm.apply(fresh_event_invite_received("c1", "alice", CallKind::Audio));
        let (sk, pk) = fresh_keypair();
        let _ = sm.apply(CallEvent::LocalAccept {
            call_id: "c1".into(),
            my_x25519_secret: sk,
            my_x25519_pub: pk,
        });
        let effects = sm.apply(CallEvent::VoiceTransportUp {
            call_id: "c1".into(),
        });
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::Notify(TransportNotification::CallConnected { .. })))
        );
        assert!(matches!(sm.get("c1"), Some(s) if matches!(s.status, CallStatus::Active)));
    }

    #[test]
    fn voice_transport_down_active_cleans_up_and_sends_end() {
        let mut sm = CallStateMachine::new();
        let _ = sm.apply(fresh_event_invite_received("c1", "alice", CallKind::Audio));
        let (sk, pk) = fresh_keypair();
        let _ = sm.apply(CallEvent::LocalAccept {
            call_id: "c1".into(),
            my_x25519_secret: sk,
            my_x25519_pub: pk,
        });
        let _ = sm.apply(CallEvent::VoiceTransportUp { call_id: "c1".into() });
        let effects = sm.apply(CallEvent::VoiceTransportDown {
            call_id: "c1".into(),
            reason: "network drop".into(),
        });
        assert!(effects.iter().any(|e| matches!(e, Effect::StopVoiceSession { .. })));
        assert!(effects.iter().any(|e| matches!(e, Effect::SendCallEnd { .. })));
        assert!(sm.get("c1").is_none());
    }

    #[test]
    fn ringing_received_only_during_outgoing() {
        let mut sm = CallStateMachine::new();
        // No outgoing — ringing is a no-op.
        let effects = sm.apply(CallEvent::RingingReceived {
            call_id: "c1".into(),
        });
        assert!(effects.is_empty());

        // Outgoing — ringing emits notification.
        let _ = sm.apply(fresh_event_local_start("c1", "bob", CallKind::Audio));
        let effects = sm.apply(CallEvent::RingingReceived {
            call_id: "c1".into(),
        });
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::Notify(TransportNotification::CallRinging { .. })))
        );
    }

    #[test]
    fn has_outgoing_to_returns_true_only_for_outgoing() {
        let mut sm = CallStateMachine::new();
        assert!(!sm.has_outgoing_to("bob"));
        let _ = sm.apply(fresh_event_local_start("c1", "bob", CallKind::Audio));
        assert!(sm.has_outgoing_to("bob"));
        assert!(!sm.has_outgoing_to("carol"));
    }

    #[test]
    fn accept_received_for_unknown_call_is_noop() {
        let mut sm = CallStateMachine::new();
        let (_, peer_pub) = fresh_keypair();
        let effects = sm.apply(CallEvent::AcceptReceived {
            call_id: "unknown".into(),
            from: "bob".into(),
            peer_x25519_pub: peer_pub,
        });
        assert!(effects.is_empty());
    }

    #[test]
    fn accept_received_validates_sender_matches_peer() {
        let mut sm = CallStateMachine::new();
        let _ = sm.apply(fresh_event_local_start("c1", "bob", CallKind::Audio));
        let (_, attacker_pub) = fresh_keypair();
        let effects = sm.apply(CallEvent::AcceptReceived {
            call_id: "c1".into(),
            from: "carol".into(), // not bob — attacker
            peer_x25519_pub: attacker_pub,
        });
        assert!(effects.is_empty(), "wrong sender rejected silently");
    }

    #[test]
    fn dialing_timeout_after_accept_is_noop() {
        // Accept transitioned the state to Connecting — timer that
        // fires later should not produce a missed-call.
        let mut sm = CallStateMachine::new();
        let _ = sm.apply(fresh_event_local_start("c1", "bob", CallKind::Audio));
        let (_, bob_pub) = fresh_keypair();
        let _ = sm.apply(CallEvent::AcceptReceived {
            call_id: "c1".into(),
            from: "bob".into(),
            peer_x25519_pub: bob_pub,
        });
        let effects = sm.apply(CallEvent::LocalDialingTimeout { call_id: "c1".into() });
        assert!(effects.is_empty());
    }

    #[test]
    fn full_call_lifecycle_caller_side() {
        let mut sm = CallStateMachine::new();
        let _ = sm.apply(fresh_event_local_start("c1", "bob", CallKind::Audio));
        assert!(matches!(sm.get("c1"), Some(s) if matches!(s.status, CallStatus::Outgoing)));

        let _ = sm.apply(CallEvent::RingingReceived { call_id: "c1".into() });
        assert!(matches!(sm.get("c1"), Some(s) if matches!(s.status, CallStatus::Outgoing)));

        let (_, bob_pub) = fresh_keypair();
        let _ = sm.apply(CallEvent::AcceptReceived {
            call_id: "c1".into(),
            from: "bob".into(),
            peer_x25519_pub: bob_pub,
        });
        assert!(matches!(sm.get("c1"), Some(s) if matches!(s.status, CallStatus::Connecting)));

        let _ = sm.apply(CallEvent::VoiceTransportUp { call_id: "c1".into() });
        assert!(matches!(sm.get("c1"), Some(s) if matches!(s.status, CallStatus::Active)));

        let _ = sm.apply(CallEvent::EndReceived {
            call_id: "c1".into(),
            reason: "peer hung up".into(),
        });
        assert!(sm.get("c1").is_none());
    }
}
