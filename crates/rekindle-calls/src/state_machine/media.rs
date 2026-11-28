//! Inputs of either side: the local end (hangup, superseded, consent,
//! `EndAll`), the peer's `CallEnd`, media up and down, consent checks, and
//! in-call media state and reactions.

use rekindle_types::subscription_events::{CallEvent, DirectCallInfo};

use super::{
    CallStateMachine, CallTimer, Effect, CALL_END_DEADLINE_MS, CALL_SUPERSEDED, CONNECTION_LOST,
    CONSENT_TIMEOUT_MS,
};
use crate::state::{CallKind, CallStatus};

impl CallStateMachine {
    /// End `call_id` here (plan CE1, decision 22 (d)): the local end comes
    /// first and never waits on the network. The call leaves, its timers
    /// stop, an unanswered invite's row is discarded, `Ended` retires the
    /// call window, its media stops, and only then is `CallEnd` committed.
    pub(super) fn end_locally(&mut self, call_id: &str, reason: &str, now_ms: u64) -> Vec<Effect> {
        let Some(held) = self.take(call_id) else {
            return vec![];
        };
        let mut effects = vec![Effect::CancelTimers {
            call_id: call_id.into(),
        }];
        if matches!(held.call.status, CallStatus::Outgoing) {
            effects.push(Effect::SettleInvite {
                call_id: call_id.into(),
                answered: false,
            });
        }
        effects.push(Effect::Emit(CallEvent::Ended {
            call_id: call_id.into(),
            reason: reason.into(),
        }));
        if held.voice_up() {
            effects.push(Effect::StopVoiceSession {
                call_id: call_id.into(),
                reason: reason.into(),
            });
        }
        effects.push(Effect::SendCallEnd {
            recipient: held.call.peer_pubkey.clone(),
            call_id: call_id.into(),
            reason: reason.into(),
            deadline_ms: now_ms.saturating_add(CALL_END_DEADLINE_MS),
        });
        effects
    }

    /// End every call with `peer` other than `keep` that is not our own
    /// Outgoing one: the peer left it, since it rang or answered `keep`
    /// (plan CE1 (22c), CE2 (30)). Its media stops before `keep`'s starts.
    pub(super) fn supersede_stale_with(
        &mut self,
        peer: &str,
        keep: &str,
        now_ms: u64,
    ) -> Vec<Effect> {
        let mut stale: Vec<String> = self
            .active
            .values()
            .filter(|h| {
                h.call.peer_pubkey == peer
                    && h.call.call_id != keep
                    && !matches!(h.call.status, CallStatus::Outgoing)
            })
            .map(|h| h.call.call_id.clone())
            .collect();
        stale.sort();
        stale
            .iter()
            .flat_map(|id| self.end_locally(id, CALL_SUPERSEDED, now_ms))
            .collect()
    }

    /// The peer hung up, in any state. Only its own media stops (plan CE2).
    pub(super) fn apply_end_received(
        &mut self,
        call_id: &str,
        from: &str,
        reason: String,
    ) -> Vec<Effect> {
        if self.held_from(call_id, from).is_none() {
            return vec![];
        }
        let Some(held) = self.take(call_id) else {
            return vec![];
        };
        let mut effects = vec![Effect::CancelTimers {
            call_id: call_id.into(),
        }];
        if held.voice_up() {
            effects.push(Effect::StopVoiceSession {
                call_id: call_id.into(),
                reason: reason.clone(),
            });
        }
        effects.push(Effect::Emit(CallEvent::Ended {
            call_id: call_id.into(),
            reason,
        }));
        effects
    }

    /// Media is up: the callee now sends its `CallAccept`; both sides go
    /// Active, start the consent window, and show the call.
    pub(super) fn apply_voice_transport_up(&mut self, call_id: &str, now_ms: u64) -> Vec<Effect> {
        let Some(held) = self.active.get_mut(call_id) else {
            return vec![];
        };
        if !matches!(held.call.status, CallStatus::Connecting) {
            return vec![];
        }
        held.call.status = CallStatus::Active;
        held.active_since_ms = Some(now_ms);
        let peer = held.call.peer_pubkey.clone();
        let kind = held.call.kind;
        let mut effects = Vec::new();
        if !held.outgoing {
            if let Some(secret) = held.call.my_x25519_secret.as_ref() {
                effects.push(Effect::SendCallAccept {
                    recipient: peer.clone(),
                    call_id: call_id.into(),
                    acceptor_x25519_pub: x25519_dalek::PublicKey::from(secret).to_bytes(),
                    video_decode_codecs: held.local_decode_codecs.clone(),
                });
            }
        }
        effects.extend([
            Effect::ArmTimer {
                call_id: call_id.into(),
                timer: CallTimer::Consent,
                at_ms: now_ms.saturating_add(CONSENT_TIMEOUT_MS),
            },
            Effect::Emit(CallEvent::Connected {
                call_id: call_id.into(),
                direct: Some(DirectCallInfo {
                    kind: super::kind_str(kind).into(),
                    peer_key: peer.clone(),
                    peer_display_name: held.display_name.clone(),
                    expected_local_camera: matches!(kind, CallKind::Video),
                }),
                group_id: None,
            }),
            Effect::PresentActiveCall {
                call_id: call_id.into(),
            },
            Effect::FocusConversation {
                call_id: call_id.into(),
                peer,
                display_name: held.display_name.clone(),
                reason: "call-accepted".into(),
            },
        ]);
        effects
    }

    /// Media failed to start or dropped. A callee whose media never started
    /// declines (its accept was never sent); otherwise the call ends here.
    pub(super) fn apply_voice_transport_down(
        &mut self,
        call_id: &str,
        reason: &str,
        now_ms: u64,
    ) -> Vec<Effect> {
        let Some(held) = self.active.get(call_id) else {
            return vec![];
        };
        if !held.voice_up() {
            return vec![];
        }
        let callee_setup = !held.outgoing && matches!(held.call.status, CallStatus::Connecting);
        if !callee_setup {
            return self.end_locally(call_id, reason, now_ms);
        }
        let Some(held) = self.take(call_id) else {
            return vec![];
        };
        vec![
            Effect::CancelTimers {
                call_id: call_id.into(),
            },
            Effect::StopVoiceSession {
                call_id: call_id.into(),
                reason: reason.into(),
            },
            Effect::SendCallDecline {
                recipient: held.call.peer_pubkey,
                call_id: call_id.into(),
                reason: format!("voice session failed: {reason}"),
            },
            Effect::Emit(CallEvent::Ended {
                call_id: call_id.into(),
                reason: reason.into(),
            }),
        ]
    }

    /// RFC 7675 §5.1: an Active call whose peer showed no media-plane proof
    /// of life for [`CONSENT_TIMEOUT_MS`] ends with [`CONNECTION_LOST`] on
    /// this side alone; otherwise the next check is armed for when consent
    /// would expire.
    pub(super) fn apply_consent_check(
        &mut self,
        call_id: &str,
        last_inbound_media_ms: Option<u64>,
        now_ms: u64,
    ) -> Vec<Effect> {
        let Some(since) = self
            .active
            .get(call_id)
            .filter(|h| matches!(h.call.status, CallStatus::Active))
            .and_then(|h| h.active_since_ms)
        else {
            return vec![];
        };
        let last = last_inbound_media_ms.map_or(since, |m| m.max(since));
        let silent = now_ms.saturating_sub(last);
        if silent < CONSENT_TIMEOUT_MS {
            return vec![Effect::ArmTimer {
                call_id: call_id.into(),
                timer: CallTimer::Consent,
                at_ms: now_ms.saturating_add(CONSENT_TIMEOUT_MS - silent),
            }];
        }
        self.end_locally(call_id, CONNECTION_LOST, now_ms)
    }

    /// End every held call (logout, quit), each through the local end.
    pub(super) fn apply_end_all(&mut self, reason: &str, now_ms: u64) -> Vec<Effect> {
        let mut ids: Vec<String> = self.active.keys().cloned().collect();
        ids.sort();
        ids.iter()
            .flat_map(|id| self.end_locally(id, reason, now_ms))
            .collect()
    }

    pub(super) fn apply_local_media_state(
        &self,
        call_id: &str,
        [audio, video, screen]: [bool; 3],
        now_ms: u64,
    ) -> Vec<Effect> {
        let Some(held) = self.active.get(call_id).filter(|h| h.voice_up()) else {
            return vec![];
        };
        vec![Effect::SendCallMediaState {
            recipient: held.call.peer_pubkey.clone(),
            call_id: call_id.into(),
            audio,
            video,
            screen,
            timestamp_ms: now_ms,
        }]
    }

    pub(super) fn apply_local_reaction(
        &self,
        call_id: &str,
        emoji: String,
        now_ms: u64,
    ) -> Vec<Effect> {
        let Some(held) = self.active.get(call_id).filter(|h| h.voice_up()) else {
            return vec![];
        };
        vec![Effect::SendCallReaction {
            recipient: held.call.peer_pubkey.clone(),
            call_id: call_id.into(),
            emoji,
            timestamp_ms: now_ms,
        }]
    }

    pub(super) fn apply_media_state_received(
        &self,
        call_id: &str,
        from: &str,
        [audio, video, screen]: [bool; 3],
        timestamp_ms: u64,
    ) -> Vec<Effect> {
        if self.held_from(call_id, from).is_none() {
            return vec![];
        }
        vec![Effect::Emit(CallEvent::MediaStateChanged {
            call_id: call_id.into(),
            audio,
            video,
            screen,
            timestamp_ms,
        })]
    }

    pub(super) fn apply_reaction_received(
        &self,
        call_id: &str,
        from: &str,
        emoji: String,
        timestamp_ms: u64,
    ) -> Vec<Effect> {
        if self.held_from(call_id, from).is_none() {
            return vec![];
        }
        vec![Effect::Emit(CallEvent::ReactionReceived {
            call_id: call_id.into(),
            sender: from.into(),
            emoji,
            timestamp_ms,
        })]
    }
}
