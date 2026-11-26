//! The interpreter port (plan E4.1 S1): what performing an [`Effect`]
//! needs from a host. The Host's call adapter implements [`CallPort`] in
//! S7 (E4.2 then swaps the sends for its per-session signaling
//! transactions behind the same port); [`interpret`] is the one mapping
//! from effects to port calls, so every host performs them alike.

use async_trait::async_trait;
use rekindle_codec::capnp_codec::direct::{
    CallAccept, CallClosed, CallInvite, CallMediaState, CallReaction, CallRinging, DirectBody,
};
use rekindle_types::subscription_events::CallEvent;

use super::{CallInput, CallTimer, Effect};
use crate::error::CallError;
use crate::state::CallKind;

/// What a host gives the call state machine's interpreter.
#[async_trait]
pub trait CallPort: Send + Sync {
    /// Now, ms since the epoch, from the host's clock port.
    fn now_ms(&self) -> u64;

    /// Commit call `call_id`'s invite (`body`) to `recipient` as an outbox
    /// row, re-sent until [`Self::settle_invite`] or `deadline_ms` (plan
    /// CE3). Returns once the row is stored.
    async fn commit_invite(
        &self,
        recipient: &str,
        call_id: &str,
        body: DirectBody,
        deadline_ms: u64,
    ) -> Result<(), CallError>;

    /// Settle call `call_id`'s invite row: `answered`, or discarded.
    async fn settle_invite(&self, call_id: &str, answered: bool);

    /// Commit a `CallEnd` (`body`) to `recipient` as an outbox row due by
    /// `deadline_ms` (plan CE1). Returns once the row is stored.
    async fn commit_end(
        &self,
        recipient: &str,
        body: DirectBody,
        deadline_ms: u64,
    ) -> Result<(), CallError>;

    /// Send one other call signal (ringing, accept, decline, media state,
    /// reaction) to `recipient` as a sealed `DmFrame`.
    async fn send(&self, recipient: &str, body: DirectBody) -> Result<(), CallError>;

    /// Arm `timer` of `call_id` at `at_ms`, replacing it if armed. When it
    /// fires, the host applies the matching input (for
    /// [`CallTimer::Consent`], with `call_id`'s latest inbound media time).
    fn arm_timer(&self, call_id: &str, timer: CallTimer, at_ms: u64);

    /// Disarm every timer of `call_id`.
    fn cancel_timers(&self, call_id: &str);

    /// Start `call_id`'s media with `peer` (its keys are read from the call
    /// state); media running for another call stops first.
    async fn start_media(&self, call_id: &str, peer: &str, kind: CallKind)
        -> Result<(), CallError>;

    /// Stop `call_id`'s media; a no-op when another call's media runs.
    async fn stop_media(&self, call_id: &str);

    /// Write a `missed_calls` row.
    async fn persist_missed_call(
        &self,
        call_id: &str,
        peer: &str,
        kind: CallKind,
        expired_at_ms: u64,
    ) -> Result<(), CallError>;

    /// Deliver an event to subscribers.
    fn emit(&self, event: CallEvent);

    /// Show the controls of a call that just connected.
    fn present_active_call(&self, call_id: &str);

    /// Bring the incoming-call surface forward.
    fn surface_call_window(&self, call_id: &str);
}

/// Perform one effect. Returns the input the outcome feeds back, if any:
/// a failed invite commit is `LocalUnreachable`, a media start is
/// `VoiceTransportUp` or `VoiceTransportDown`.
pub async fn interpret<P: CallPort + ?Sized>(port: &P, effect: Effect) -> Option<CallInput> {
    match effect {
        Effect::SendCallInvite {
            recipient,
            call_id,
            offer_kind,
            initiator_pubkey,
            initiator_x25519_pub,
            expires_at_ms,
            video_decode_codecs,
        } => {
            let body = DirectBody::CallInvite(CallInvite {
                call_id: call_id.clone(),
                offer_kind,
                initiator_pubkey,
                initiator_x25519_pub: initiator_x25519_pub.to_vec(),
                expires_at_ms,
                video_decode_codecs,
            });
            let committed = port
                .commit_invite(&recipient, &call_id, body, expires_at_ms)
                .await;
            committed.err().map(|e| CallInput::LocalUnreachable {
                call_id,
                reason: format!("call invite not committed: {e}"),
                now_ms: port.now_ms(),
            })
        }
        Effect::SendCallEnd {
            recipient,
            call_id,
            reason,
            deadline_ms,
        } => {
            let body = DirectBody::CallEnd(CallClosed {
                call_id: call_id.clone(),
                reason,
            });
            if let Err(e) = port.commit_end(&recipient, body, deadline_ms).await {
                tracing::warn!(call = %call_id, peer = %recipient, error = %e,
                    "CallEnd not committed to the outbox; the peer ends on its own timeout");
            }
            None
        }
        Effect::StartVoiceSession {
            call_id,
            peer,
            kind,
        } => {
            let started = port.start_media(&call_id, &peer, kind).await;
            let now_ms = port.now_ms();
            Some(match started {
                Ok(()) => CallInput::VoiceTransportUp { call_id, now_ms },
                Err(e) => CallInput::VoiceTransportDown {
                    call_id,
                    reason: e.to_string(),
                    now_ms,
                },
            })
        }
        Effect::PersistMissedCall {
            call_id,
            peer_pubkey,
            kind,
            expired_at_ms,
        } => {
            let persisted = port
                .persist_missed_call(&call_id, &peer_pubkey, kind, expired_at_ms)
                .await;
            if let Err(e) = persisted {
                tracing::warn!(call = %call_id, error = %e, "missed call not recorded");
            }
            None
        }
        other => {
            perform_plain(port, other).await;
            None
        }
    }
}

/// The effects whose outcome feeds nothing back (`interpret` performs the rest).
async fn perform_plain<P: CallPort + ?Sized>(port: &P, effect: Effect) {
    match effect {
        Effect::SendCallRinging { recipient, call_id } => {
            let body = DirectBody::CallRinging(CallRinging { call_id });
            send_logged(port, &recipient, body).await;
        }
        Effect::SendCallAccept {
            recipient,
            call_id,
            acceptor_x25519_pub,
            video_decode_codecs,
        } => {
            let body = DirectBody::CallAccept(CallAccept {
                call_id,
                acceptor_x25519_pub: acceptor_x25519_pub.to_vec(),
                video_decode_codecs,
            });
            send_logged(port, &recipient, body).await;
        }
        Effect::SendCallDecline {
            recipient,
            call_id,
            reason,
        } => {
            let body = DirectBody::CallDecline(CallClosed { call_id, reason });
            send_logged(port, &recipient, body).await;
        }
        Effect::SendCallMediaState {
            recipient,
            call_id,
            audio,
            video,
            screen,
            timestamp_ms,
        } => {
            let body = DirectBody::CallMediaState(CallMediaState {
                call_id,
                audio,
                video,
                screen,
                timestamp_ms,
            });
            send_logged(port, &recipient, body).await;
        }
        Effect::SendCallReaction {
            recipient,
            call_id,
            emoji,
            timestamp_ms,
        } => {
            let body = DirectBody::CallReaction(CallReaction {
                call_id,
                emoji,
                timestamp_ms,
            });
            send_logged(port, &recipient, body).await;
        }
        Effect::SettleInvite { call_id, answered } => port.settle_invite(&call_id, answered).await,
        Effect::StopVoiceSession { call_id, .. } => port.stop_media(&call_id).await,
        Effect::ArmTimer {
            call_id,
            timer,
            at_ms,
        } => port.arm_timer(&call_id, timer, at_ms),
        Effect::CancelTimers { call_id } => port.cancel_timers(&call_id),
        Effect::Emit(event) => port.emit(event),
        Effect::PresentActiveCall { call_id } => port.present_active_call(&call_id),
        Effect::SurfaceCallWindow { call_id } => port.surface_call_window(&call_id),
        Effect::FocusConversation {
            call_id,
            peer,
            display_name,
            reason,
        } => port.emit(CallEvent::FocusConversation {
            call_id,
            peer_key: peer,
            peer_display_name: display_name,
            reason,
        }),
        Effect::SendCallInvite { .. }
        | Effect::SendCallEnd { .. }
        | Effect::StartVoiceSession { .. }
        | Effect::PersistMissedCall { .. } => {
            // Effects with an outcome: `interpret` performs these itself
            // and never hands them here.
        }
    }
}

/// A best-effort send, as the live signaling path does today (E4.2 turns
/// these into acknowledged transactions with a typed failure).
async fn send_logged<P: CallPort + ?Sized>(port: &P, recipient: &str, body: DirectBody) {
    if let Err(e) = port.send(recipient, body).await {
        tracing::debug!(peer = %recipient, error = %e, "call signal send failed");
    }
}
