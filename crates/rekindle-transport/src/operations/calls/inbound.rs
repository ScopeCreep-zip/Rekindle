//! Inbound event entry points (called from W16.7's dispatch).
//!
//! These mirror the local actions but are driven by inbound
//! envelopes / timer firings. The W16.7 receive dispatch pulls
//! (call_id, sender, payload) out of the wire envelope, builds the
//! appropriate CallInput, and calls the matching dispatch helper.

use rekindle_calls::{CallInput, CallKind};
use rekindle_types::notification::TransportNotification;
use rekindle_utils::timestamp_ms;
use tracing::{debug, warn};

use super::CallRuntime;
use crate::payload::dm::DmPayload;

impl CallRuntime {
    /// W16.7 entry: an inbound CallInvite arrived. Drives state
    /// machine to Incoming.
    pub async fn handle_invite_received(
        &self,
        call_id: String,
        from: String,
        from_display_name: String,
        kind: CallKind,
        peer_x25519_pub: [u8; 32],
        expires_at_ms: u64,
    ) {
        let event = CallInput::InviteReceived {
            call_id,
            from,
            from_display_name,
            kind,
            peer_x25519_pub,
            expires_at_ms,
            received_at_ms: timestamp_ms(),
        };
        let effects = self.inner.state_machine.lock().apply(event);
        self.interpret_effects(effects).await;
    }

    /// W16.7 entry: an inbound CallAccept arrived (caller-side).
    pub async fn handle_accept_received(
        &self,
        call_id: String,
        from: String,
        peer_x25519_pub: [u8; 32],
    ) {
        let event = CallInput::AcceptReceived {
            call_id,
            from,
            peer_x25519_pub,
        };
        let effects = self.inner.state_machine.lock().apply(event);
        self.interpret_effects(effects).await;
    }

    /// W16.7 entry: an inbound CallDecline arrived (caller-side).
    pub async fn handle_decline_received(&self, call_id: String, reason: String) {
        let event = CallInput::DeclineReceived { call_id, reason };
        let effects = self.inner.state_machine.lock().apply(event);
        self.interpret_effects(effects).await;
    }

    /// W16.7 entry: an inbound CallRinging arrived (caller-side
    /// alerting hint).
    pub async fn handle_ringing_received(&self, call_id: String) {
        let event = CallInput::RingingReceived { call_id };
        let effects = self.inner.state_machine.lock().apply(event);
        self.interpret_effects(effects).await;
    }

    /// W16.7 entry: an inbound CallEnd arrived from either side.
    pub async fn handle_end_received(&self, call_id: String, reason: String) {
        let event = CallInput::EndReceived { call_id, reason };
        let effects = self.inner.state_machine.lock().apply(event);
        self.interpret_effects(effects).await;
    }

    /// W16.7 entry: voice transport finished bringing up audio.
    pub async fn handle_voice_transport_up(&self, call_id: String) {
        let event = CallInput::VoiceTransportUp { call_id };
        let effects = self.inner.state_machine.lock().apply(event);
        self.interpret_effects(effects).await;
    }

    /// W16.7 entry: voice transport failed mid-call.
    pub async fn handle_voice_transport_down(&self, call_id: String, reason: String) {
        let event = CallInput::VoiceTransportDown { call_id, reason };
        let effects = self.inner.state_machine.lock().apply(event);
        self.interpret_effects(effects).await;
    }

    /// W16.7 — route a [`DmPayload`] received via `InboundHandler::on_dm`
    /// to the matching `handle_*` method. Returns `true` if the payload
    /// was a call-signaling variant (handled by the runtime); `false` if
    /// it was a non-call DmPayload (caller routes elsewhere — DM body,
    /// friend-add, presence, etc.).
    ///
    /// Implementers' typical shape:
    /// ```ignore
    /// async fn on_dm(&self, sender, payload, ts, seq, correlation_id) {
    ///     if !self.seq_tracker.check_and_record(...).await? { return; }
    ///     if self.call_runtime.route_dm_payload(sender, payload, ts).await {
    ///         return;
    ///     }
    ///     // Fall through to friend-add / DM body / etc.
    /// }
    /// ```
    ///
    /// This consumes `payload` (since each handler arm needs ownership
    /// to extract the call_id, x25519 pub, etc.). On false, the caller
    /// has lost the original payload — but in practice, on `true` the
    /// runtime handled it; on `false`, the caller would have processed
    /// it themselves anyway. If a caller needs to inspect-then-decide,
    /// they can pattern-match before calling this helper.
    pub async fn route_dm_payload(
        &self,
        sender: &crate::handler::VerifiedSender,
        payload: DmPayload,
    ) -> bool {
        match payload {
            // W16.5b — CallInvite + CallRinging no longer travel via
            // app_message; the wire-level invite-and-ringing handshake
            // uses Veilid `app_call`. Receive-side routes through
            // `handle_inbound_call_invite` from the `on_call` dispatch
            // (subscriptions/dispatch.rs). Caller-side `RingingReceived`
            // is fed back from the synchronous `app_call` reply by
            // `dispatch_call_invite`.
            DmPayload::CallAccept {
                call_id,
                acceptor_x25519_pub,
            } => {
                let Ok(pub_arr) = <[u8; 32]>::try_from(acceptor_x25519_pub.as_slice()) else {
                    warn!(
                        sender = %sender.public_key,
                        "CallAccept: bad x25519 pub length, dropping"
                    );
                    return true;
                };
                self.handle_accept_received(call_id, sender.public_key.clone(), pub_arr)
                    .await;
                true
            }
            DmPayload::CallDecline { call_id, reason } => {
                self.handle_decline_received(call_id, reason).await;
                true
            }
            DmPayload::CallEnd { call_id, reason } => {
                self.handle_end_received(call_id, reason).await;
                true
            }
            DmPayload::CallMediaState {
                call_id,
                audio,
                video,
                screen,
                timestamp_ms,
            } => {
                // Mid-call media state changes don't drive the state
                // machine — they're transparent UI hints. Emit the
                // notification directly. Drop if the call is unknown.
                if self.inner.state_machine.lock().get(&call_id).is_some() {
                    self.inner.notifications.notify(
                        &TransportNotification::CallMediaStateChanged {
                            call_id,
                            audio,
                            video,
                            screen,
                            timestamp_ms,
                        },
                    );
                }
                true
            }
            DmPayload::CallReaction {
                call_id,
                emoji,
                timestamp_ms,
            } => {
                const MAX_EMOJI_BYTES: usize = 32;
                if emoji.len() > MAX_EMOJI_BYTES {
                    debug!(
                        sender = %sender.public_key,
                        bytes = emoji.len(),
                        "CallReaction: oversized emoji, dropping"
                    );
                    return true;
                }
                if self.inner.state_machine.lock().get(&call_id).is_some() {
                    self.inner
                        .notifications
                        .notify(&TransportNotification::CallReactionReceived {
                            call_id,
                            sender: sender.public_key.clone(),
                            emoji,
                            timestamp_ms,
                        });
                }
                true
            }
            // Group call signaling (W16.13 — separate runtime), DM
            // invite request/reply (W16.10b — uses
            // `EnvelopeQueue::deliver_reply`), and all other non-call
            // DmPayload variants are unhandled by the 1:1 call runtime.
            // The implementer routes them: group calls to their
            // group-call runtime, DM invites via the queue's
            // expect-reply oneshot registry, the rest to friend-add /
            // DM body / presence handlers.
            _ => false,
        }
    }

    /// W16.5b — receive-side handler for `InboundCall::CallInvite`.
    /// Drives the state machine with `CallInput::InviteReceived`,
    /// interprets resulting effects (PersistCallState, SpawnIncomingTimer,
    /// Notify(IncomingCall)), and synthesizes the synchronous
    /// `CallResponse::CallRinging` reply.
    ///
    /// The caller (`InboundHandler::on_call`) returns this as the
    /// `app_call_reply` payload, which travels back to the sender as the
    /// `CallRinging` confirmation within Veilid's 5–10 s RPC budget.
    pub async fn handle_inbound_call_invite(
        &self,
        sender_pubkey: &str,
        sender_display_name: &str,
        invite: crate::payload::rpc::CallInvitePayload,
    ) -> crate::payload::rpc::CallResponse {
        use crate::payload::rpc::{CallResponse, CallRingingPayload};

        let kind = match invite.offer_kind {
            0 => CallKind::Audio,
            1 => CallKind::Video,
            other => {
                warn!(other, "handle_inbound_call_invite: invalid offer_kind");
                return CallResponse::Rejected {
                    reason: format!("invalid offer_kind: {other}"),
                };
            }
        };
        if invite.initiator_x25519_pub.len() != 32 {
            warn!(
                len = invite.initiator_x25519_pub.len(),
                "handle_inbound_call_invite: bad x25519 pub length"
            );
            return CallResponse::Rejected {
                reason: "bad initiator_x25519_pub length".into(),
            };
        }
        let mut peer_pub = [0u8; 32];
        peer_pub.copy_from_slice(&invite.initiator_x25519_pub);

        let now = timestamp_ms();
        let event = CallInput::InviteReceived {
            call_id: invite.call_id.clone(),
            from: sender_pubkey.to_string(),
            from_display_name: sender_display_name.to_string(),
            kind,
            peer_x25519_pub: peer_pub,
            expires_at_ms: invite.expires_at_ms,
            received_at_ms: now,
        };
        let effects = self.inner.state_machine.lock().apply(event);
        if effects.is_empty() {
            // Duplicate invite for an existing call_id, etc. The state
            // machine is idempotent — return the same CallRinging reply
            // so the caller's perception is consistent.
            debug!(
                call_id = invite.call_id,
                "duplicate CallInvite — replying CallRinging"
            );
            return CallResponse::CallRinging(CallRingingPayload {
                call_id: invite.call_id,
            });
        }
        self.interpret_effects(effects).await;
        CallResponse::CallRinging(CallRingingPayload {
            call_id: invite.call_id,
        })
    }
}
