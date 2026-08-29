//! Outgoing (caller-side) call-event handlers.
//!
//! [`super::CallStateMachine::apply`] dispatches here for the caller-side
//! events: starting a call, cancelling, dialing timeout, unreachable,
//! accept/decline/ringing received.

use rekindle_types::notification::TransportNotification;
use x25519_dalek::StaticSecret;

use super::{kind_str, CallStateMachine, Effect, StartCallParams};
use crate::state::{CallState, CallStatus};

impl CallStateMachine {
    pub(super) fn apply_local_start_call(&mut self, params: StartCallParams) -> Vec<Effect> {
        let StartCallParams {
            call_id,
            peer,
            peer_display_name,
            kind,
            my_x25519_secret,
            my_x25519_pub,
            expires_at_ms,
            started_at_ms,
        } = params;
        let call_id = call_id.as_str();
        let peer = peer.as_str();
        let state = CallState {
            call_id: call_id.into(),
            peer_pubkey: peer.into(),
            kind,
            status: CallStatus::Outgoing,
            expires_at_ms,
            my_x25519_secret: Some(my_x25519_secret),
            peer_x25519_pub: None,
            call_key: None,
            peer_video_decode_codecs: Vec::new(),
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

    pub(super) fn apply_local_cancel(&mut self, call_id: &str, reason: String) -> Vec<Effect> {
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

    pub(super) fn apply_local_dialing_timeout(&mut self, call_id: &str) -> Vec<Effect> {
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
    pub(super) fn apply_local_unreachable(&mut self, call_id: &str, reason: String) -> Vec<Effect> {
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

    pub(super) fn apply_accept_received(
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
                Effect::CancelTimer {
                    call_id: cid.clone(),
                },
                Effect::SendCallEnd {
                    recipient: peer,
                    call_id: cid.clone(),
                    reason: "missing local x25519 secret".into(),
                },
                Effect::DeletePersistedCall {
                    call_id: cid.clone(),
                },
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
                    Effect::CancelTimer {
                        call_id: cid.clone(),
                    },
                    Effect::SendCallEnd {
                        recipient: peer,
                        call_id: cid.clone(),
                        reason: format!("call_key derive failed: {e}"),
                    },
                    Effect::DeletePersistedCall {
                        call_id: cid.clone(),
                    },
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

    pub(super) fn apply_decline_received(&mut self, call_id: &str, reason: String) -> Vec<Effect> {
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

    pub(super) fn apply_ringing_received(&self, call_id: &str) -> Vec<Effect> {
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
}
