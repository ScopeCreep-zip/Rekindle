//! Order the local join snapshot with the session's live state changes.

use parking_lot::Mutex;
use rekindle_types::subscription_events::{JoinHandshake, VoiceEvent, VoiceMediaReady, VoiceScope};

use crate::media_ready::MediaReadyInputs;

#[derive(Default)]
struct State {
    inputs: MediaReadyInputs,
    last_media_ready: Option<VoiceMediaReady>,
    ended: bool,
}

impl State {
    fn media_transition(&mut self) -> Option<VoiceMediaReady> {
        let current = self.inputs.snapshot();
        if self.last_media_ready.as_ref() == Some(&current) {
            return None;
        }
        self.last_media_ready = Some(current.clone());
        Some(current)
    }
}

/// The transport and desktop video gate share this state for one session.
/// The emitters synchronously publish to the Host journal under the same
/// lock that protects the snapshot and all handshake/readiness changes.
#[derive(Default)]
pub struct VoiceSessionState {
    state: Mutex<State>,
}

impl VoiceSessionState {
    pub fn handshake(&self) -> JoinHandshake {
        self.state.lock().inputs.handshake
    }

    pub(crate) fn advance_handshake(
        &self,
        next: JoinHandshake,
        emit: impl FnOnce(Option<VoiceMediaReady>),
    ) -> bool {
        let mut state = self.state.lock();
        let advances = match next {
            JoinHandshake::Announced => false,
            JoinHandshake::Seen => state.inputs.handshake == JoinHandshake::Announced,
            JoinHandshake::Connected => state.inputs.handshake != JoinHandshake::Connected,
        };
        if state.ended || !advances {
            return false;
        }
        state.inputs.handshake = next;
        emit(state.media_transition());
        true
    }

    pub fn emit_local_joined(&self, scope: VoiceScope, emit: impl FnOnce(VoiceEvent)) {
        let state = self.state.lock();
        if !state.ended {
            emit(VoiceEvent::LocalJoined {
                scope,
                join_handshake: state.inputs.handshake,
                media_ready: state.inputs.snapshot(),
            });
        }
    }

    pub fn update_media_ready(
        &self,
        update: impl FnOnce(&mut MediaReadyInputs),
        emit: impl FnOnce(VoiceMediaReady),
    ) {
        let mut state = self.state.lock();
        if !state.ended {
            update(&mut state.inputs);
            if let Some(transition) = state.media_transition() {
                emit(transition);
            }
        }
    }

    pub fn media_ready_gate(&self) -> Result<(), String> {
        let state = self.state.lock();
        if state.ended {
            Err("not-in-voice".to_string())
        } else if state.inputs.ready() {
            Ok(())
        } else {
            Err(state.inputs.block_reason().to_string())
        }
    }

    pub fn end(&self, emit: impl FnOnce(VoiceMediaReady)) {
        let mut state = self.state.lock();
        state.ended = true;
        if state
            .last_media_ready
            .take()
            .is_some_and(|status| status.ready)
        {
            emit(VoiceMediaReady {
                ready: false,
                reason: "left-channel".to_string(),
            });
        }
    }
}

#[cfg(test)]
#[path = "session_state_tests.rs"]
mod tests;
