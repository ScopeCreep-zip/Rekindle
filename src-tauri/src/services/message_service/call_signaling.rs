//! Phase 23.a — call signaling dispatcher.
//!
//! Pre-port lived inline in `services/message_service.rs` (~170 LoC).
//! Routes incoming `MessagePayload::Call*` variants to the
//! `rekindle_calls::signaling::handlers::*` orchestrators via a
//! per-message `CallsAdapter`. Lift into a sibling module so the
//! parent `mod.rs` stays focused on the higher-level inbound
//! dispatch + the public send API.
//!
//! Phase 14 architecture: 1:1 call signaling handlers ported into
//! `rekindle-calls/src/signaling/handlers/*`. The adapter maps
//! `CallSignalEvent` → src-tauri `ChatEvent` / `NotificationEvent`
//! for UI emission.

use std::sync::Arc;

use rekindle_protocol::messaging::envelope::MessagePayload;

use crate::channels::ChatEvent;
use crate::db::DbPool;
use crate::state::AppState;

pub(super) async fn handle_call_signaling_payload(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &DbPool,
    sender_hex: &str,
    payload: MessagePayload,
) {
    let adapter = crate::services::calls_adapter::CallsAdapter::new(
        state.clone(),
        app_handle.clone(),
        pool.clone(),
    );
    let deps = adapter.as_ref();
    match payload {
        MessagePayload::CallInvite {
            call_id,
            offer_kind,
            initiator_pubkey,
            initiator_x25519_pub,
            expires_at_ms,
        } => {
            rekindle_calls::signaling::handlers::handle_incoming_invite(
                deps,
                sender_hex,
                &call_id,
                offer_kind,
                &initiator_pubkey,
                &initiator_x25519_pub,
                expires_at_ms,
            )
            .await;
        }
        MessagePayload::CallAccept {
            call_id,
            acceptor_x25519_pub,
        } => {
            rekindle_calls::signaling::handlers::handle_accept_received(
                deps,
                sender_hex,
                &call_id,
                &acceptor_x25519_pub,
            )
            .await;
        }
        MessagePayload::CallDecline { call_id, reason } => {
            rekindle_calls::signaling::handlers::handle_decline_received(
                deps, sender_hex, &call_id, reason,
            );
        }
        MessagePayload::CallRinging { call_id } => {
            rekindle_calls::signaling::handlers::handle_ringing_received(
                deps, sender_hex, &call_id,
            );
        }
        // Wave 13 W13.9 — hangup / cancel. Works for any state
        // (Outgoing / Incoming / Connecting / Active) so a
        // cancel-while-ringing or decline-after-accept-race cleans
        // up both sides cleanly.
        MessagePayload::CallEnd { call_id, reason } => {
            // W15.2 — capture status before remove so we know
            // whether voice was actually running. Tear it down if so.
            let (removed, was_voice_up) =
                state
                    .active_calls
                    .remove(&call_id)
                    .map_or((false, false), |c| {
                        let voice_up = matches!(
                            c.status,
                            rekindle_calls::CallStatus::Active
                                | rekindle_calls::CallStatus::Connecting
                        );
                        (true, voice_up)
                    });
            if removed {
                if was_voice_up {
                    crate::services::voice_adapter::shutdown_voice(
                        state,
                        &rekindle_voice::VoiceShutdownOpts::FULL,
                    )
                    .await;
                }
                crate::event_dispatch::emit_live(
                    app_handle,
                    "chat-event",
                    &ChatEvent::CallEnded {
                        call_id: call_id.clone(),
                        reason: reason.clone(),
                    },
                );
                tracing::info!(call_id = %call_id, %reason, "remote ended DM call");
            } else {
                tracing::debug!(call_id = %call_id, "CallEnd for unknown call_id; ignoring");
            }
        }
        // Wave 12 W12.6 — peer toggled their mic / camera / screen.
        // Emit a chat-event so the frontend's ActiveCallPanel /
        // VideoCallPanel can mount/unmount tiles reactively. Drop
        // if the call_id isn't ours so the receiver doesn't get
        // spurious UI changes for a call that already ended.
        MessagePayload::CallMediaState {
            call_id,
            audio,
            video,
            screen,
            timestamp_ms,
        } => {
            let known = state.active_calls.contains(&call_id);
            if known {
                crate::event_dispatch::emit_live(
                    app_handle,
                    "chat-event",
                    &ChatEvent::CallMediaStateChanged {
                        call_id,
                        audio,
                        video,
                        screen,
                        timestamp_ms,
                    },
                );
            } else {
                tracing::debug!(call_id = %call_id, "CallMediaState for unknown call; ignoring");
            }
        }
        // Wave 12 W12.11 — peer fired an in-call emoji reaction.
        // Cap emoji length and drop unknown call_ids so we don't
        // surface floating reactions for calls that already ended
        // (or were never accepted).
        MessagePayload::CallReaction {
            call_id,
            emoji,
            timestamp_ms,
        } => {
            const MAX_EMOJI_BYTES: usize = 32;
            if emoji.len() > MAX_EMOJI_BYTES {
                tracing::debug!(
                    "dropping CallReaction with oversized emoji ({} bytes)",
                    emoji.len(),
                );
            } else if state.active_calls.contains(&call_id) {
                crate::event_dispatch::emit_live(
                    app_handle,
                    "chat-event",
                    &ChatEvent::CallReactionReceived {
                        call_id,
                        sender: sender_hex.to_string(),
                        emoji,
                        timestamp_ms,
                    },
                );
            }
        }
        _ => {
            // Caller restricts payloads via the outer match;
            // unreachable in normal flow. Dropping silently for safety.
            tracing::debug!("handle_call_signaling_payload: non-call payload reached helper");
        }
    }
}
