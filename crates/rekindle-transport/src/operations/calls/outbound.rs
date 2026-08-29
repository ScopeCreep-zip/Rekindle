//! Local user actions: start, accept, decline, end a call, and mid-call
//! media-state / reaction pings.

use rekindle_calls::{fresh_keypair, CallEvent, CallKind, CallStatus};
use rekindle_utils::timestamp_ms;

use super::util::generate_call_id;
use super::{CallError, CallRuntime, RING_DURATION_MS};
use crate::envelope_store::EnvelopeKind;
use crate::payload::dm::{serialize_dm, DmPayload};

impl CallRuntime {
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
    pub async fn decline_dm_call(&self, call_id: &str, reason: &str) -> Result<(), CallError> {
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
    pub async fn end_dm_call(&self, call_id: &str, reason: &str) -> Result<(), CallError> {
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
    pub async fn send_call_reaction(&self, call_id: &str, emoji: &str) -> Result<(), CallError> {
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
}
