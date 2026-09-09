//! Effect interpreter: turns [`Effect`]s produced by the state machine
//! into envelope sends, timer spawns, persistence, notifications, and
//! voice-session start/stop. Also the `app_call` CallInvite dispatch
//! path.

use std::time::Duration;

use rekindle_calls::{CallInput, Effect};
use rekindle_utils::timestamp_ms;
use tracing::{debug, warn};

use super::util::{classify_call_invite_error, kind_str, status_str};
use super::{CallRuntime, EffectsFuture, TimerKind, CALL_INVITE_RPC_TIMEOUT_MS};
use crate::envelope_store::{EnvelopeKind, PersistedCallState};
use crate::payload::dm::{serialize_dm, DmPayload};

impl CallRuntime {
    // ── Effect interpreter ──────────────────────────────────────────

    /// Process every effect produced by the state machine. Effects
    /// can recursively produce more effects (e.g. a voice-session
    /// start failure produces a `VoiceTransportDown` event whose
    /// effects also need interpretation), handled by re-entry in the
    /// async task. The `'static` future captures `self` by clone (the
    /// runtime is `Arc`-internal so the clone is cheap).
    pub(super) fn interpret_effects(&self, effects: Vec<Effect>) -> EffectsFuture {
        let runtime = self.clone();
        Box::pin(async move {
            for effect in effects {
                runtime.interpret_one(effect).await;
            }
        })
    }

    async fn interpret_one(&self, effect: Effect) {
        match effect {
            Effect::SendCallInvite {
                recipient,
                call_id,
                offer_kind,
                initiator_x25519_pub,
                expires_at_ms,
            } => {
                // W16.5b — dispatch CallInvite via Veilid `app_call`
                // (5–10 s budget; matches SIP 100-Trying / 180-Ringing).
                // Receiver replies synchronously inside `app_call_reply`
                // with `CallResponse::CallRinging { call_id }`. Failures
                // map to `CallInput::LocalUnreachable` so the caller's
                // UI surfaces "peer unreachable" within ~10 s instead
                // of waiting for the 30 s ring timer.
                self.dispatch_call_invite(
                    &recipient,
                    &call_id,
                    offer_kind,
                    initiator_x25519_pub,
                    expires_at_ms,
                )
                .await;
            }
            Effect::SendCallAccept {
                recipient,
                call_id,
                acceptor_x25519_pub,
            } => {
                let payload = DmPayload::CallAccept {
                    call_id: call_id.clone(),
                    acceptor_x25519_pub: acceptor_x25519_pub.to_vec(),
                };
                self.queue_call_envelope(&recipient, &payload, EnvelopeKind::CallAccept, &call_id)
                    .await;
            }
            Effect::SendCallDecline {
                recipient,
                call_id,
                reason,
            } => {
                let payload = DmPayload::CallDecline {
                    call_id: call_id.clone(),
                    reason,
                };
                self.queue_call_envelope(&recipient, &payload, EnvelopeKind::CallDecline, &call_id)
                    .await;
            }
            Effect::SendCallEnd {
                recipient,
                call_id,
                reason,
            } => {
                let payload = DmPayload::CallEnd {
                    call_id: call_id.clone(),
                    reason,
                };
                self.queue_call_envelope(&recipient, &payload, EnvelopeKind::CallEnd, &call_id)
                    .await;
            }
            // W16.5b — Effect::SendCallRinging dropped: the CallRinging
            // reply is synthesized synchronously by `on_call` into the
            // app_call_reply payload, not produced as an outbound
            // envelope. The receiver-side state machine no longer emits
            // this effect.
            Effect::StartVoiceSession {
                call_id,
                peer,
                kind,
                call_key,
            } => {
                if let Err(reason) = self
                    .inner
                    .voice_launcher
                    .start_voice_session(&call_id, &peer, kind, call_key)
                    .await
                {
                    warn!(
                        call_id,
                        peer, reason, "voice session start failed; tearing down call"
                    );
                    let event = CallInput::VoiceTransportDown {
                        call_id: call_id.clone(),
                        reason,
                    };
                    let cleanup = self.inner.state_machine.lock().apply(event);
                    // Recursive re-entry: cleanup may itself send a
                    // CallEnd envelope, persist deletion, emit
                    // notification. Avoid stack-overflow by bounding
                    // recursion to one level (cleanup never re-enters
                    // StartVoiceSession).
                    self.interpret_effects(cleanup).await;
                }
            }
            Effect::StopVoiceSession { call_id, reason } => {
                self.inner
                    .voice_launcher
                    .stop_voice_session(&call_id, &reason)
                    .await;
            }
            Effect::SpawnDialingTimer {
                call_id,
                expires_at_ms,
            } => {
                self.spawn_timeout(call_id, expires_at_ms, TimerKind::Dialing);
            }
            Effect::SpawnIncomingTimer {
                call_id,
                expires_at_ms,
            } => {
                self.spawn_timeout(call_id, expires_at_ms, TimerKind::Incoming);
            }
            Effect::CancelTimer { call_id } => {
                if let Some(handle) = self.inner.timers.lock().remove(&call_id) {
                    handle.abort();
                }
            }
            Effect::PersistCallState {
                call_id,
                peer_pubkey,
                kind,
                status,
                expires_at_ms,
                my_x25519_secret,
                peer_x25519_pub,
            } => {
                let state = PersistedCallState {
                    owner_key: self.inner.owner_key.clone(),
                    call_id,
                    peer_pubkey,
                    kind: kind_str(kind).into(),
                    status: status_str(status).into(),
                    expires_at_ms,
                    my_x25519_secret: my_x25519_secret.map(|b| b.to_vec()),
                    peer_x25519_pub: peer_x25519_pub.map(|b| b.to_vec()),
                    group_participants: vec![],
                    inserted_at_ms: timestamp_ms(),
                };
                if let Err(e) = self.inner.store.save_active_call(state).await {
                    warn!(error = %e, "PersistCallState failed");
                }
            }
            Effect::DeletePersistedCall { call_id } => {
                if let Err(e) = self
                    .inner
                    .store
                    .delete_active_call(&self.inner.owner_key, &call_id)
                    .await
                {
                    warn!(error = %e, "DeletePersistedCall failed");
                }
                // Drop any pending outbound envelopes for this call_id
                // so retries don't fire against a dead call.
                let _ = self.inner.queue.cancel_by_correlation(&call_id).await;
            }
            Effect::PersistMissedCall { .. } => {
                // missed_calls persistence is shell-specific (Tauri
                // uses a SQLite table; CLI/daemon may log to stdout
                // or skip). Default: emit a debug log here; the
                // upcoming `EnvelopeStore` extension (W16 follow-up)
                // adds a typed `record_missed_call` method.
                debug!("PersistMissedCall: not yet wired to EnvelopeStore (W16 follow-up)");
            }
            Effect::Notify(notif) => {
                self.inner.notifications.notify(&notif);
            }
        }
    }

    async fn queue_call_envelope(
        &self,
        recipient: &str,
        payload: &DmPayload,
        kind: EnvelopeKind,
        correlation_id: &str,
    ) {
        let bytes = match serialize_dm(payload) {
            Ok(b) => b,
            Err(e) => {
                warn!(error = %e, "queue_call_envelope: serialize_dm failed");
                return;
            }
        };
        if let Err(e) = self
            .inner
            .queue
            .send(recipient, bytes, kind, Some(correlation_id))
            .await
        {
            warn!(error = %e, "queue_call_envelope: queue send failed");
        }
    }

    /// W16.5b — dispatch a CallInvite via Veilid `app_call` and feed
    /// the result back into the state machine.
    ///
    /// On success, the receiver's `on_call` handler returns
    /// `CallResponse::CallRinging { call_id }`; we drive
    /// `CallInput::RingingReceived` so the caller's UI flips
    /// "Calling…" → "Ringing…" on real evidence.
    ///
    /// On failure, we classify the transport error and drive
    /// `CallInput::LocalUnreachable { reason }` so the state machine
    /// drops the Outgoing state, cancels the dialing timer, and emits
    /// `TransportNotification::CallUnreachable` for the UI.
    async fn dispatch_call_invite(
        &self,
        recipient: &str,
        call_id: &str,
        offer_kind: u8,
        initiator_x25519_pub: [u8; 32],
        expires_at_ms: u64,
    ) {
        use crate::frame::TypeId;
        use crate::payload::rpc::{CallInvitePayload, CallResponse};

        let invite = CallInvitePayload {
            call_id: call_id.to_string(),
            offer_kind,
            initiator_x25519_pub: initiator_x25519_pub.to_vec(),
            expires_at_ms,
        };
        let payload_bytes = match postcard::to_stdvec(&invite) {
            Ok(b) => b,
            Err(e) => {
                warn!(error = %e, "dispatch_call_invite: serialize failed");
                self.feed_unreachable(call_id, "send_failed").await;
                return;
            }
        };

        let result = self
            .inner
            .queue
            .send_app_call(
                recipient,
                TypeId::CallInvite,
                &payload_bytes,
                Duration::from_millis(CALL_INVITE_RPC_TIMEOUT_MS),
            )
            .await;

        match result {
            Ok(reply_bytes) => {
                // Parse the receiver's CallResponse::CallRinging.
                match postcard::from_bytes::<CallResponse>(&reply_bytes) {
                    Ok(CallResponse::CallRinging(ringing)) if ringing.call_id == call_id => {
                        let event = CallInput::RingingReceived {
                            call_id: call_id.to_string(),
                        };
                        let effects = self.inner.state_machine.lock().apply(event);
                        self.interpret_effects(effects).await;
                    }
                    Ok(other) => {
                        warn!(
                            ?other,
                            call_id, "dispatch_call_invite: unexpected CallResponse variant"
                        );
                        self.feed_unreachable(call_id, "send_failed").await;
                    }
                    Err(e) => {
                        warn!(error = %e, call_id, "dispatch_call_invite: reply parse failed");
                        self.feed_unreachable(call_id, "send_failed").await;
                    }
                }
            }
            Err(e) => {
                let reason = classify_call_invite_error(&e);
                debug!(error = %e, call_id, reason, "dispatch_call_invite: app_call failed");
                self.feed_unreachable(call_id, reason).await;
            }
        }
    }

    /// Helper: feed `CallInput::LocalUnreachable` into the state machine
    /// and interpret resulting effects.
    async fn feed_unreachable(&self, call_id: &str, reason: &str) {
        let event = CallInput::LocalUnreachable {
            call_id: call_id.to_string(),
            reason: reason.to_string(),
        };
        let effects = self.inner.state_machine.lock().apply(event);
        self.interpret_effects(effects).await;
    }
}
