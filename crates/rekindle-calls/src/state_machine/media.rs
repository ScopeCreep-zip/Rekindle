//! Media/end-of-call call-event handlers.
//!
//! [`super::CallStateMachine::apply`] dispatches here for the events that
//! apply regardless of caller/receiver direction: end received, voice
//! transport up/down.

use rekindle_types::notification::TransportNotification;

use super::{kind_str, CallStateMachine, Effect};
use crate::state::{CallKind, CallStatus};

impl CallStateMachine {
    pub(super) fn apply_end_received(&mut self, call_id: &str, reason: String) -> Vec<Effect> {
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

    pub(super) fn apply_voice_transport_up(&mut self, call_id: &str) -> Vec<Effect> {
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

    pub(super) fn apply_voice_transport_down(
        &mut self,
        call_id: &str,
        reason: String,
    ) -> Vec<Effect> {
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
