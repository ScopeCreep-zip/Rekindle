//! Phase 23.D — incoming-message dispatch lifted from
//! `message_service/mod.rs`. Owns the parse → verify → decrypt →
//! per-`MessagePayload`-variant fan-out pipeline (architecture §13:
//! receive-side authorization gate + ephemeral filter + friend gate)
//! plus the small `app_call` reply handler for DM invites and relay
//! offers. Friend-handshake side effects live in `friend_handlers`;
//! transport / outgoing in `transport` + `outgoing`. Phase 23.D split
//! the original 714-LoC flat file into focused submodules.

mod handlers;
mod prepare;

use std::sync::Arc;

use rekindle_protocol::messaging::envelope::MessagePayload;

use crate::channels::ChatEvent;
use crate::db::DbPool;
use crate::state::AppState;

use super::call_signaling::handle_call_signaling_payload;
use super::friend_handlers::{
    handle_friend_accept_full, handle_friend_reject, handle_friend_request_full,
    handle_profile_key_rotated, handle_unfriended, handle_unfriended_ack, IncomingFriendAccept,
    IncomingFriendRequest,
};
use super::session_reset::handle_session_reset_payload;

use handlers::{handle_channel_message, handle_direct_message};
use prepare::prepare_incoming;

/// Result of parsing, decrypting, and validating an incoming Veilid message.
struct PreparedMessage {
    sender_hex: String,
    payload: MessagePayload,
    timestamp: i64,
}

/// hand back. Returns `None` if the message wasn't one of those payloads
/// (so the caller can fall through to the generic message handler).
///
/// Architecture §27.1 line 2916 — `app_call` is the spec'd transport
/// for DM invites because the initiator needs a confirmed reply.
/// Architecture §13.2 step 2 — same model for RelayOffer, where Carol
/// needs to know Bob actually persisted her relay route.
pub async fn try_handle_dm_invite_app_call(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &DbPool,
    raw_message: &[u8],
) -> Option<Vec<u8>> {
    let prepared = prepare_incoming(app_handle, state, pool, raw_message).await?;
    match prepared.payload {
        MessagePayload::RelayOffer {
            relay_route_blob,
            relay_pseudonym,
        } => {
            let ingest = crate::services::relay::add_received_offer(
                state,
                pool,
                &relay_pseudonym,
                &relay_route_blob,
            )
            .await;
            let reply = match ingest {
                Ok(()) => {
                    crate::services::relay::pool::republish_relay_pool(state, pool).await;
                    MessagePayload::RelayOfferAck {
                        ok: true,
                        reason: String::new(),
                    }
                }
                Err(e) => MessagePayload::RelayOfferAck {
                    ok: false,
                    reason: e,
                },
            };
            Some(serde_json::to_vec(&reply).unwrap_or_else(|_| b"ACK".to_vec()))
        }
        MessagePayload::DmInvite {
            record_key,
            slot_seed,
            alice_pseudonym,
            alice_subkey,
            bob_subkey,
        } => {
            let result = crate::services::dm::handle_incoming_dm_invite(
                app_handle,
                state,
                pool,
                &prepared.sender_hex,
                &record_key,
                &slot_seed,
                &alice_pseudonym,
                alice_subkey,
                bob_subkey,
            )
            .await;
            let reply = match result {
                Ok(()) => MessagePayload::DmAccept {
                    record_key: record_key.clone(),
                },
                Err(e) => MessagePayload::DmDecline {
                    record_key: record_key.clone(),
                    reason: e,
                },
            };
            Some(serde_json::to_vec(&reply).unwrap_or_else(|_| b"ACK".to_vec()))
        }
        // Wave 13 — call signaling no longer travels via app_call
        // (CallInvite/Accept/Decline/Ringing all dispatch via
        // process_envelope's app_message arms). DmInvite + RelayOffer
        // are the only remaining app_call payloads in this codebase.
        _ => None,
    }
}

/// Handle an incoming message from the Veilid network.
///
/// Flow: parse envelope → verify signature → decrypt if session exists →
/// parse payload → dispatch by type (DM, friend request, typing, etc.)
#[allow(
    clippy::too_many_lines,
    reason = "Top-level payload dispatch — each MessagePayload arm has its own helper; splitting further would obscure the dispatch table."
)]
pub async fn handle_incoming_message(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &DbPool,
    raw_message: &[u8],
) {
    let Some(msg) = prepare_incoming(app_handle, state, pool, raw_message).await else {
        return;
    };

    match msg.payload {
        MessagePayload::DirectMessage { body, .. } => {
            handle_direct_message(
                app_handle,
                state,
                pool,
                &msg.sender_hex,
                &body,
                msg.timestamp,
            );
        }
        MessagePayload::ChannelMessage {
            channel_id, body, ..
        } => {
            handle_channel_message(
                app_handle,
                state,
                pool,
                &msg.sender_hex,
                &channel_id,
                &body,
                msg.timestamp,
            );
        }
        MessagePayload::TypingIndicator { typing } => {
            crate::event_dispatch::emit_live(
                app_handle,
                "chat-event",
                &ChatEvent::TypingIndicator {
                    from: msg.sender_hex,
                    typing,
                },
            );
        }
        MessagePayload::FriendRequest {
            display_name,
            message,
            prekey_bundle,
            profile_dht_key,
            route_blob,
            mailbox_dht_key,
            invite_id,
        } => {
            let req = IncomingFriendRequest {
                sender_hex: &msg.sender_hex,
                display_name: &display_name,
                message: &message,
                prekey_bundle: &prekey_bundle,
                profile_dht_key: &profile_dht_key,
                route_blob: &route_blob,
                mailbox_dht_key: &mailbox_dht_key,
                invite_id: invite_id.as_deref(),
            };
            handle_friend_request_full(app_handle, state, pool, &req).await;
        }
        MessagePayload::FriendAccept {
            prekey_bundle,
            profile_dht_key,
            route_blob,
            mailbox_dht_key,
            ephemeral_key,
            signed_prekey_id,
            one_time_prekey_id,
            ml_kem_ciphertext,
            used_ot_pqpk_id,
        } => {
            let accept = IncomingFriendAccept {
                sender_hex: &msg.sender_hex,
                prekey_bundle: &prekey_bundle,
                profile_dht_key: &profile_dht_key,
                route_blob,
                mailbox_dht_key: &mailbox_dht_key,
                ephemeral_key: &ephemeral_key,
                signed_prekey_id,
                one_time_prekey_id,
                ml_kem_ciphertext: &ml_kem_ciphertext,
                used_ot_pqpk_id,
            };
            handle_friend_accept_full(app_handle, state, pool, &accept).await;
        }
        MessagePayload::FriendReject => {
            handle_friend_reject(app_handle, state, pool, &msg.sender_hex);
        }
        MessagePayload::FriendRequestReceived => {
            crate::event_dispatch::emit_live(
                app_handle,
                "chat-event",
                &ChatEvent::FriendRequestDelivered { to: msg.sender_hex },
            );
        }
        MessagePayload::ProfileKeyRotated {
            new_profile_dht_key,
        } => {
            handle_profile_key_rotated(state, pool, &msg.sender_hex, &new_profile_dht_key).await;
        }
        MessagePayload::PresenceUpdate { .. } => {}
        MessagePayload::Unfriended => {
            handle_unfriended(app_handle, state, pool, &msg.sender_hex).await;
        }
        MessagePayload::UnfriendedAck => {
            handle_unfriended_ack(state, pool, &msg.sender_hex);
        }
        MessagePayload::RelayOffer {
            relay_route_blob,
            relay_pseudonym,
        } => {
            // Bob receives Carol's offer (architecture §13.2 step 2).
            // Persist the blob and republish the (padded) pool so peers
            // failing direct delivery have a fallback path.
            if let Err(e) = crate::services::relay::add_received_offer(
                state,
                pool,
                &relay_pseudonym,
                &relay_route_blob,
            )
            .await
            {
                tracing::warn!(error = %e, from = %msg.sender_hex, "failed to persist RelayOffer");
            } else {
                crate::services::relay::pool::republish_relay_pool(state, pool).await;
            }
        }
        MessagePayload::RelayWithdraw { relay_pseudonym } => {
            if let Err(e) =
                crate::services::relay::remove_received_offer(state, pool, &relay_pseudonym).await
            {
                tracing::warn!(error = %e, from = %msg.sender_hex, "failed to drop revoked RelayOffer");
            } else {
                crate::services::relay::pool::republish_relay_pool(state, pool).await;
            }
        }
        MessagePayload::RelayEnvelope {
            target_pubkey,
            inner_payload,
        } => {
            // Carol forwards (architecture §13.3 step 3).
            if let Err(e) = crate::services::relay::handle_relay_envelope(
                state,
                pool,
                &target_pubkey,
                &inner_payload,
            )
            .await
            {
                tracing::debug!(error = %e, target = %target_pubkey, "RelayEnvelope dropped");
            }
        }
        MessagePayload::DmInvite {
            record_key,
            slot_seed,
            alice_pseudonym,
            alice_subkey,
            bob_subkey,
        } => {
            if let Err(e) = crate::services::dm::handle_incoming_dm_invite(
                app_handle,
                state,
                pool,
                &msg.sender_hex,
                &record_key,
                &slot_seed,
                &alice_pseudonym,
                alice_subkey,
                bob_subkey,
            )
            .await
            {
                tracing::warn!(error = %e, from = %msg.sender_hex, "failed to ingest DmInvite");
            }
        }
        MessagePayload::DmAccept { record_key: _ } => {
            // DmAccept is the reply to a DmInvite app_call; if it
            // arrives via app_message instead (peer used the wrong
            // path) we just log and ignore — Alice's outbound
            // app_call already resolved with the inline reply.
            tracing::trace!("received DmAccept via app_message; ignoring");
        }
        // Wave 13 W13.4 — peer is calling us. Dispatch into the new
        // services::calls state machine which inserts CallState=Incoming,
        // emits ChatEvent::IncomingCall, surfaces the window, and fires
        // CallRinging back as an alerting ack.
        // Wave 13 — call signaling group: invite/accept/decline/ringing
        // /end/media-state/reaction. Extracted into a helper to keep the
        // dispatcher under the workspace too_many_lines budget. The
        // helper itself goes away in W16.6 when call signaling moves
        // into rekindle-transport::operations::calls.
        payload @ (MessagePayload::CallInvite { .. }
        | MessagePayload::CallAccept { .. }
        | MessagePayload::CallDecline { .. }
        | MessagePayload::CallRinging { .. }
        | MessagePayload::CallEnd { .. }
        | MessagePayload::CallMediaState { .. }
        | MessagePayload::CallReaction { .. }) => {
            handle_call_signaling_payload(app_handle, state, pool, &msg.sender_hex, payload).await;
        }
        // Wave 12 W12.9 + Wave 13 W13.13 — all group call signaling now
        // travels via app_message (was app_call in W12.9; flipped in
        // W13.13 to match 1:1 calls). The group_calls dispatcher
        // routes Offer → handle_incoming_group_invite, Accept →
        // handle_group_accept_received, Decline →
        // handle_group_decline_received, ParticipantJoined/Left →
        // gossip-style chat-event re-emit.
        payload @ (MessagePayload::GroupCallOffer { .. }
        | MessagePayload::GroupCallAccept { .. }
        | MessagePayload::GroupCallDecline { .. }
        | MessagePayload::GroupCallParticipantJoined { .. }
        | MessagePayload::GroupCallParticipantLeft { .. }) => {
            crate::services::calls_adapter::handle_group_call_payload(
                app_handle,
                state,
                pool,
                &msg.sender_hex,
                payload,
            )
            .await;
        }
        // P3.3 — Signal session-renewal payload trio. Extracted into a
        // helper so the main dispatcher stays under the workspace
        // too_many_lines budget. See handle_session_reset_payload below.
        payload @ (MessagePayload::SessionResetRequest { .. }
        | MessagePayload::SessionResetAccept { .. }
        | MessagePayload::SessionResetDecline { .. }) => {
            handle_session_reset_payload(app_handle, state, &msg.sender_hex, payload);
        }
        MessagePayload::RelayOfferAck { ok: _, reason: _ } => {
            tracing::trace!("received RelayOfferAck via app_message; ignoring");
        }
        MessagePayload::DmDecline { record_key, reason: _ } => {
            if let Err(e) =
                crate::services::dm::handle_incoming_dm_decline(state, pool, &record_key).await
            {
                tracing::debug!(error = %e, "DmDecline drop");
            }
        }
        MessagePayload::GroupDmInvite {
            record_key,
            slot_seed,
            initiator_pseudonym,
            participants_json,
            wrapped_mek,
            mek_generation,
        } => {
            if let Err(e) = crate::services::dm::handle_incoming_group_dm_invite(
                app_handle,
                state,
                pool,
                &msg.sender_hex,
                &record_key,
                &slot_seed,
                &initiator_pseudonym,
                &participants_json,
                &wrapped_mek,
                mek_generation,
            )
            .await
            {
                tracing::warn!(error = %e, from = %msg.sender_hex, "failed to ingest GroupDmInvite");
            }
        }
        MessagePayload::DmLeave { record_key } => {
            if let Err(e) = crate::services::dm::handle_incoming_dm_leave(
                state,
                pool,
                &msg.sender_hex,
                &record_key,
            )
            .await
            {
                tracing::debug!(error = %e, "DmLeave drop");
            }
        }
        MessagePayload::RegisterPushRelay { .. }
        | MessagePayload::UnregisterPushRelay { .. } => {
            // The push-relay daemon is a separate binary
            // (`rekindle-push-relay`). Desktop clients don't *receive*
            // these — they only send. Drop silently if we somehow get
            // one.
            tracing::debug!("ignoring push-relay register payload at desktop client");
        }
        MessagePayload::WakeNotify { ts } => {
            crate::services::push_relay::handle_wake_notify(state, ts);
        }
        MessagePayload::StatusRequest { target_pubkey } => {
            // Architecture §13.5 — relay friends serve cached presence.
            // Only respond if (a) the requester is a friend (don't leak
            // status to strangers) and (b) we relay for the target.
            if let Err(e) = crate::services::relay::respond_to_status_request(
                state,
                pool,
                &msg.sender_hex,
                &target_pubkey,
            )
            .await
            {
                tracing::trace!(error = %e, "StatusRequest dropped");
            }
        }
        MessagePayload::StatusResponse {
            target_pubkey,
            status,
            status_message,
            last_seen,
            route_blob,
        } => {
            crate::services::relay::handle_status_response(
                state,
                &target_pubkey,
                &status,
                status_message.as_deref(),
                last_seen,
                &route_blob,
            );
        }
        MessagePayload::DmVideoFragment {
            stream_id,
            frame_seq,
            fragment_index,
            fragment_count,
            keyframe,
            timestamp,
            chunk,
        } => {
            // W11.4 — accumulate fragments and emit a `videoFrame`
            // event when the last chunk lands. The Signal layer has
            // already verified authenticity (sender's identity key is
            // bound to the envelope), so we don't repeat per-fragment
            // signatures the way community video does.
            if let Some(frame) = state.dm_video_reassembly.record_fragment(
                &msg.sender_hex,
                stream_id,
                frame_seq,
                fragment_index,
                fragment_count,
                keyframe,
                timestamp,
                chunk,
            ) {
                // Phase 13 — assembled-frame event now flows through
                // DmDeps::emit_event(DmEvent::VideoFrameAssembled); the
                // adapter handles the base64 + json layout for the
                // `dm-video-frame` Tauri emit.
                let adapter = crate::services::dm_adapter::DmAdapter::new(
                    Arc::clone(state),
                    app_handle.clone(),
                    pool.clone(),
                );
                rekindle_dm::video::dispatch_assembled_frame(
                    &*adapter,
                    &msg.sender_hex,
                    frame,
                );
            }
        }
    }
}
