//! Caller-side inputs: placing a call, its ring timeout, an unreachable
//! peer, and the callee's ringing, accept and decline.

use rekindle_types::subscription_events::CallEvent;

use super::{kind_str, CallStateMachine, CallTimer, Effect, Held, OutgoingCall};
use crate::signaling::outbound::RING_DURATION_MS;
use crate::state::{CallState, CallStatus};

impl CallStateMachine {
    pub(super) fn apply_local_start_call(&mut self, call: OutgoingCall) -> Vec<Effect> {
        let OutgoingCall {
            call_id,
            peer,
            peer_display_name,
            kind,
            my_x25519_secret,
            video_decode_codecs,
            now_ms,
        } = call;
        let expires_at_ms = now_ms.saturating_add(RING_DURATION_MS);
        let my_pub = x25519_dalek::PublicKey::from(&my_x25519_secret).to_bytes();
        let state = CallState {
            call_id: call_id.clone(),
            peer_pubkey: peer.clone(),
            kind,
            status: CallStatus::Outgoing,
            expires_at_ms,
            my_x25519_secret: Some(my_x25519_secret),
            peer_x25519_pub: None,
            media_secret: None,
            media_sender: std::sync::Arc::new(rekindle_secrets::sframe::SframeSender::fresh()),
            // Filled from the peer's CallAccept.
            peer_video_decode_codecs: Vec::new(),
        };
        self.active.insert(
            call_id.clone(),
            Held {
                call: state,
                display_name: peer_display_name.clone(),
                outgoing: true,
                local_decode_codecs: video_decode_codecs.clone(),
                active_since_ms: None,
            },
        );
        vec![
            Effect::ArmTimer {
                call_id: call_id.clone(),
                timer: CallTimer::Dialing,
                at_ms: expires_at_ms,
            },
            Effect::SendCallInvite {
                recipient: peer.clone(),
                call_id: call_id.clone(),
                offer_kind: kind.as_u8(),
                initiator_pubkey: self.own_key.clone(),
                initiator_x25519_pub: my_pub,
                expires_at_ms,
                video_decode_codecs,
            },
            Effect::Emit(CallEvent::Started {
                call_id: call_id.clone(),
                kind: kind_str(kind).into(),
                peer_key: peer.clone(),
                peer_display_name: peer_display_name.clone(),
                expires_at_ms,
            }),
            Effect::FocusConversation {
                call_id,
                peer,
                display_name: peer_display_name,
                reason: "call-started".into(),
            },
        ]
    }

    /// The caller's ring expired unanswered: a missed-call row and `TimedOut`.
    pub(super) fn apply_dialing_timeout(&mut self, call_id: &str, now_ms: u64) -> Vec<Effect> {
        let due = self.active.get(call_id).is_some_and(|h| {
            matches!(h.call.status, CallStatus::Outgoing) && now_ms >= h.call.expires_at_ms
        });
        let Some(held) = due.then(|| self.take(call_id)).flatten() else {
            return vec![];
        };
        vec![
            Effect::PersistMissedCall {
                call_id: call_id.into(),
                peer_pubkey: held.call.peer_pubkey.clone(),
                kind: held.call.kind,
                expired_at_ms: held.call.expires_at_ms,
            },
            Effect::Emit(CallEvent::TimedOut {
                call_id: call_id.into(),
            }),
        ]
    }

    /// The invite never left (no session, no route): no missed-call row,
    /// the peer never saw it.
    pub(super) fn apply_unreachable(&mut self, call_id: &str, reason: String) -> Vec<Effect> {
        let outgoing = self
            .active
            .get(call_id)
            .is_some_and(|h| matches!(h.call.status, CallStatus::Outgoing));
        if !outgoing || self.take(call_id).is_none() {
            return vec![];
        }
        vec![
            Effect::CancelTimers {
                call_id: call_id.into(),
            },
            Effect::SettleInvite {
                call_id: call_id.into(),
                answered: false,
            },
            Effect::Emit(CallEvent::Unreachable {
                call_id: call_id.into(),
                reason,
            }),
        ]
    }

    pub(super) fn apply_accept_received(
        &mut self,
        call_id: &str,
        from: &str,
        peer_x25519_pub: &[u8],
        video_decode_codecs: Vec<String>,
        now_ms: u64,
    ) -> Vec<Effect> {
        let valid = self
            .held_from(call_id, from)
            .is_some_and(|h| matches!(h.call.status, CallStatus::Outgoing));
        if !valid || peer_x25519_pub.len() != 32 {
            return vec![];
        }
        // Plan CE3: the answer settles our invite's row. Plan CE2: the peer
        // answering this call has left any other call with us.
        let mut effects = vec![Effect::SettleInvite {
            call_id: call_id.into(),
            answered: true,
        }];
        effects.extend(self.supersede_stale_with(from, call_id, now_ms));
        let derived = self.active.get(call_id).and_then(|h| {
            let secret = h.call.my_x25519_secret.as_ref()?;
            Some(crate::derive_call_key(secret, peer_x25519_pub, call_id))
        });
        let key = match derived {
            Some(Ok(key)) => key,
            Some(Err(e)) => {
                effects.extend(self.end_locally(call_id, &format!("derive call_key: {e}"), now_ms));
                return effects;
            }
            None => {
                effects.extend(self.end_locally(call_id, "missing local x25519 secret", now_ms));
                return effects;
            }
        };
        let Some(held) = self.active.get_mut(call_id) else {
            return effects;
        };
        let mut peer_arr = [0u8; 32];
        peer_arr.copy_from_slice(peer_x25519_pub);
        held.call.peer_x25519_pub = Some(peer_arr);
        held.call.media_secret = Some(zeroize::Zeroizing::new(key));
        held.call.peer_video_decode_codecs = video_decode_codecs;
        held.call.status = CallStatus::Connecting;
        effects.push(Effect::CancelTimers {
            call_id: call_id.into(),
        });
        effects.push(Effect::StartVoiceSession {
            call_id: call_id.into(),
            peer: from.into(),
            kind: held.call.kind,
        });
        effects
    }

    pub(super) fn apply_decline_received(
        &mut self,
        call_id: &str,
        from: &str,
        reason: String,
    ) -> Vec<Effect> {
        let outgoing = self
            .held_from(call_id, from)
            .is_some_and(|h| matches!(h.call.status, CallStatus::Outgoing));
        if !outgoing || self.take(call_id).is_none() {
            return vec![];
        }
        vec![
            Effect::CancelTimers {
                call_id: call_id.into(),
            },
            Effect::SettleInvite {
                call_id: call_id.into(),
                answered: true,
            },
            Effect::Emit(CallEvent::Declined {
                call_id: call_id.into(),
                reason,
            }),
        ]
    }

    /// The callee holds our invite and rings: the row stops re-sending and
    /// our panel flips "Calling…" → "Ringing…".
    pub(super) fn apply_ringing_received(&self, call_id: &str, from: &str) -> Vec<Effect> {
        let outgoing = self
            .held_from(call_id, from)
            .is_some_and(|h| matches!(h.call.status, CallStatus::Outgoing));
        if !outgoing {
            return vec![];
        }
        vec![
            Effect::SettleInvite {
                call_id: call_id.into(),
                answered: true,
            },
            Effect::Emit(CallEvent::Ringing {
                call_id: call_id.into(),
            }),
        ]
    }
}
