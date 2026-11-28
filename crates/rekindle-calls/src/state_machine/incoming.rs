//! Callee-side inputs: an invite (temp-mute, repeats, busy, superseded,
//! glare), the local accept and decline, and the ring timeout.

use crate::signaling::handlers::BUSY;
use rekindle_types::subscription_events::CallEvent;
use x25519_dalek::StaticSecret;

use super::{
    kind_str, CallStateMachine, CallTimer, Effect, Held, InboundInvite, GLARE_OTHER_LOSES,
    GLARE_RESOLVED, GLARE_WE_LOST, INVALID_X25519, TEMP_MUTED_DECLINE,
};
use crate::state::{CallState, CallStatus};

fn decline(recipient: &str, call_id: &str, reason: &str) -> Effect {
    Effect::SendCallDecline {
        recipient: recipient.into(),
        call_id: call_id.into(),
        reason: reason.into(),
    }
}

impl CallStateMachine {
    pub(super) fn apply_invite_received(&mut self, invite: InboundInvite) -> Vec<Effect> {
        let InboundInvite {
            call_id,
            from,
            from_display_name,
            kind,
            peer_x25519_pub,
            media_sender,
            expires_at_ms,
            video_decode_codecs,
            temp_muted,
            now_ms,
        } = invite;
        let Ok(peer_arr) = <[u8; 32]>::try_from(peer_x25519_pub.as_slice()) else {
            return vec![decline(&from, &call_id, INVALID_X25519)];
        };
        if temp_muted {
            return vec![decline(&from, &call_id, TEMP_MUTED_DECLINE)];
        }
        let mut effects = match self.make_room_for_invite(&from, &call_id, now_ms) {
            Ok(effects) => effects,
            Err(effects) => return effects,
        };
        // W13.15 glare: our own outgoing call to the caller. The lower
        // identity key keeps its outgoing call (E4.2 replaces the rule).
        let outgoing = self
            .active
            .values()
            .find(|h| h.call.peer_pubkey == from && matches!(h.call.status, CallStatus::Outgoing))
            .map(|h| h.call.call_id.clone());
        if let Some(ours) = outgoing {
            if self.own_key.as_str() < from.as_str() {
                effects.push(decline(&from, &call_id, GLARE_OTHER_LOSES));
                return effects;
            }
            effects.extend(self.cancel_outgoing_for_glare(&ours, now_ms));
        }
        self.active.insert(
            call_id.clone(),
            Held {
                call: CallState {
                    call_id: call_id.clone(),
                    peer_pubkey: from.clone(),
                    kind,
                    status: CallStatus::Incoming,
                    expires_at_ms,
                    my_x25519_secret: None,
                    peer_x25519_pub: Some(peer_arr),
                    media_secret: None,
                    media_sender,
                    peer_video_decode_codecs: video_decode_codecs,
                },
                display_name: from_display_name.clone(),
                outgoing: false,
                local_decode_codecs: Vec::new(),
                active_since_ms: None,
            },
        );
        effects.extend([
            Effect::Emit(CallEvent::Incoming {
                call_id: call_id.clone(),
                from: from.clone(),
                display_name: from_display_name,
                kind: kind_str(kind).into(),
                participants: Vec::new(),
                is_group: false,
                expires_at_ms,
                group_id: None,
            }),
            Effect::SurfaceCallWindow {
                call_id: call_id.clone(),
            },
            Effect::SendCallRinging {
                recipient: from,
                call_id: call_id.clone(),
            },
            Effect::ArmTimer {
                call_id,
                timer: CallTimer::Incoming,
                at_ms: expires_at_ms,
            },
        ]);
        effects
    }

    /// Plan CE1 (22c): whether the invite may ring. `Err` carries the
    /// effects of an invite that does not: a repeat of a held call re-sends
    /// Ringing (our first may be lost), a re-sent invite of an ended call
    /// is dropped, and an up call with another peer answers busy. `Ok`
    /// carries the ends of stale calls with the same peer (superseded).
    fn make_room_for_invite(
        &mut self,
        from: &str,
        call_id: &str,
        now_ms: u64,
    ) -> Result<Vec<Effect>, Vec<Effect>> {
        if let Some(held) = self.active.get(call_id) {
            let rering =
                held.call.peer_pubkey == from && matches!(held.call.status, CallStatus::Incoming);
            return Err(if rering {
                vec![Effect::SendCallRinging {
                    recipient: from.into(),
                    call_id: call_id.into(),
                }]
            } else {
                vec![]
            });
        }
        if self.ended.contains_key(call_id) {
            return Err(vec![]);
        }
        let busy = self
            .active
            .values()
            .any(|h| h.call.peer_pubkey != from && h.voice_up());
        if busy {
            return Err(vec![decline(from, call_id, BUSY)]);
        }
        Ok(self.supersede_stale_with(from, call_id, now_ms))
    }

    /// End our outgoing call lost to glare: the row is discarded, the call
    /// ends here, and `CallEnd` follows.
    fn cancel_outgoing_for_glare(&mut self, call_id: &str, now_ms: u64) -> Vec<Effect> {
        let Some(held) = self.take(call_id) else {
            return vec![];
        };
        let peer = held.call.peer_pubkey.clone();
        let mut effects = vec![
            Effect::CancelTimers {
                call_id: call_id.into(),
            },
            Effect::SettleInvite {
                call_id: call_id.into(),
                answered: false,
            },
        ];
        if held.voice_up() {
            effects.push(Effect::StopVoiceSession {
                call_id: call_id.into(),
                reason: GLARE_RESOLVED.into(),
            });
        }
        effects.push(Effect::SendCallEnd {
            recipient: peer,
            call_id: call_id.into(),
            reason: GLARE_WE_LOST.into(),
            deadline_ms: now_ms.saturating_add(super::CALL_END_DEADLINE_MS),
        });
        effects.push(Effect::Emit(CallEvent::Ended {
            call_id: call_id.into(),
            reason: GLARE_RESOLVED.into(),
        }));
        effects
    }

    /// The user accepted: derive the call key and start media. Our
    /// `CallAccept` goes out once media is up (`VoiceTransportUp`), so we
    /// are ready for the caller's first packets.
    pub(super) fn apply_local_accept(
        &mut self,
        call_id: &str,
        my_x25519_secret: StaticSecret,
        video_decode_codecs: Vec<String>,
    ) -> Vec<Effect> {
        let Some(held) = self.active.get(call_id) else {
            return vec![];
        };
        let Some(peer_pub) = held
            .call
            .peer_x25519_pub
            .filter(|_| matches!(held.call.status, CallStatus::Incoming))
        else {
            return vec![];
        };
        let key = match crate::derive_call_key(&my_x25519_secret, &peer_pub, call_id) {
            Ok(key) => key,
            Err(e) => {
                let peer = held.call.peer_pubkey.clone();
                self.take(call_id);
                let reason = format!("call_key derive failed: {e}");
                return vec![
                    Effect::CancelTimers {
                        call_id: call_id.into(),
                    },
                    decline(&peer, call_id, &reason),
                    Effect::Emit(CallEvent::Ended {
                        call_id: call_id.into(),
                        reason,
                    }),
                ];
            }
        };
        let Some(held) = self.active.get_mut(call_id) else {
            return vec![];
        };
        held.call.my_x25519_secret = Some(my_x25519_secret);
        held.call.media_secret = Some(zeroize::Zeroizing::new(key));
        held.call.status = CallStatus::Connecting;
        held.local_decode_codecs = video_decode_codecs;
        vec![
            Effect::CancelTimers {
                call_id: call_id.into(),
            },
            Effect::StartVoiceSession {
                call_id: call_id.into(),
                peer: held.call.peer_pubkey.clone(),
                kind: held.call.kind,
            },
        ]
    }

    pub(super) fn apply_local_decline(&mut self, call_id: &str, reason: String) -> Vec<Effect> {
        let incoming = self
            .active
            .get(call_id)
            .is_some_and(|h| matches!(h.call.status, CallStatus::Incoming));
        let Some(held) = incoming.then(|| self.take(call_id)).flatten() else {
            return vec![];
        };
        vec![
            Effect::CancelTimers {
                call_id: call_id.into(),
            },
            decline(&held.call.peer_pubkey, call_id, &reason),
            Effect::Emit(CallEvent::Declined {
                call_id: call_id.into(),
                reason,
            }),
        ]
    }

    /// The callee's ring expired unanswered: a missed-call row and `Missed`.
    pub(super) fn apply_incoming_timeout(&mut self, call_id: &str, now_ms: u64) -> Vec<Effect> {
        let due = self.active.get(call_id).is_some_and(|h| {
            matches!(h.call.status, CallStatus::Incoming) && now_ms >= h.call.expires_at_ms
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
            Effect::Emit(CallEvent::Missed {
                call_id: call_id.into(),
                from: held.call.peer_pubkey,
            }),
        ]
    }
}
