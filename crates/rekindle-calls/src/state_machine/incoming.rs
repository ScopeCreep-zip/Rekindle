//! Incoming (receiver-side) call-event handlers.
//!
//! [`super::CallStateMachine::apply`] dispatches here for the receiver-side
//! events: invite received, local accept/decline, incoming timeout.

use rekindle_types::notification::TransportNotification;
use x25519_dalek::StaticSecret;

use super::{kind_str, CallStateMachine, Effect, InviteReceivedParams};
use crate::state::{CallState, CallStatus};

impl CallStateMachine {
    pub(super) fn apply_invite_received(&mut self, params: InviteReceivedParams) -> Vec<Effect> {
        let InviteReceivedParams {
            call_id,
            from,
            from_display_name,
            kind,
            peer_x25519_pub,
            expires_at_ms,
            received_at_ms,
        } = params;
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
            peer_video_decode_codecs: Vec::new(),
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

    pub(super) fn apply_local_accept(
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
                    Effect::CancelTimer {
                        call_id: cid.clone(),
                    },
                    Effect::SendCallDecline {
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

    pub(super) fn apply_local_decline(&mut self, call_id: &str, reason: String) -> Vec<Effect> {
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

    pub(super) fn apply_local_incoming_timeout(&mut self, call_id: &str) -> Vec<Effect> {
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
}
