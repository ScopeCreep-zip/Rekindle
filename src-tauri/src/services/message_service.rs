use std::sync::Arc;

use rand::RngCore as _;
use rekindle_protocol::messaging::envelope::MessagePayload;
use rekindle_protocol::messaging::receiver::{parse_payload, process_incoming};
use rekindle_protocol::messaging::sender::{build_envelope_from_secret, send_envelope};
use tauri::Emitter;

use crate::channels::ChatEvent;
use crate::db::DbPool;
use crate::db_helpers::{db_call, db_call_or_default, db_fire};
use crate::state::AppState;
use crate::state_helpers;

/// Consolidated parameters for an incoming friend request.
struct IncomingFriendRequest<'a> {
    sender_hex: &'a str,
    display_name: &'a str,
    message: &'a str,
    prekey_bundle: &'a [u8],
    profile_dht_key: &'a str,
    route_blob: &'a [u8],
    mailbox_dht_key: &'a str,
    invite_id: Option<&'a str>,
}

/// Consolidated parameters for an incoming friend accept.
struct IncomingFriendAccept<'a> {
    sender_hex: &'a str,
    prekey_bundle: &'a [u8],
    profile_dht_key: &'a str,
    route_blob: Vec<u8>,
    mailbox_dht_key: &'a str,
    ephemeral_key: &'a [u8],
    signed_prekey_id: u32,
    one_time_prekey_id: Option<u32>,
}

/// Result of parsing, decrypting, and validating an incoming Veilid message.
struct PreparedMessage {
    sender_hex: String,
    payload: MessagePayload,
    timestamp: i64,
}

/// Detect a DM invite or RelayOffer arriving via `app_call`, run the
/// ingest, and return the serialized reply bytes the dispatcher should
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
        // Plan §Failure 5 — direct call offer arrives via `app_call`.
        // The frontend prompts the user to accept or decline; that
        // decision flows back through `services::call_signaling` and
        // becomes the inline reply payload here.
        MessagePayload::CallOffer {
            call_id,
            offer_kind,
            initiator_pubkey,
            initiator_x25519_pub,
            expires_at_ms,
        } => {
            let reply = crate::commands::calls::handle_incoming_offer(
                app_handle,
                state,
                pool,
                &prepared.sender_hex,
                &call_id,
                offer_kind,
                &initiator_pubkey,
                &initiator_x25519_pub,
                expires_at_ms,
            )
            .await;
            Some(serde_json::to_vec(&reply).unwrap_or_else(|_| b"ACK".to_vec()))
        }
        _ => None,
    }
}

/// Handle an incoming message from the Veilid network.
///
/// Flow: parse envelope → verify signature → decrypt if session exists →
/// parse payload → dispatch by type (DM, friend request, typing, etc.)
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
            let _ = app_handle.emit(
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
            };
            handle_friend_accept_full(app_handle, state, pool, &accept).await;
        }
        MessagePayload::FriendReject => {
            handle_friend_reject(app_handle, state, pool, &msg.sender_hex);
        }
        MessagePayload::FriendRequestReceived => {
            let _ = app_handle.emit(
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
        // Plan §Failure 5 — Call signaling lives on `app_call`. If a
        // peer mis-sends one of these via `app_message` we just trace.
        MessagePayload::CallOffer { .. }
        | MessagePayload::CallAccept { .. }
        | MessagePayload::CallDecline { .. } => {
            tracing::trace!("received call signaling via app_message; ignoring");
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
    }
}

/// Parse envelope, check block list, decrypt, deserialize payload, and filter non-friends.
///
/// Returns `None` (with appropriate logging) for any rejection reason.
async fn prepare_incoming(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &DbPool,
    raw_message: &[u8],
) -> Option<PreparedMessage> {
    let envelope = match process_incoming(raw_message) {
        Ok(env) => env,
        Err(e) => {
            tracing::error!(error = %e, "failed to parse/verify incoming message envelope");
            return None;
        }
    };

    let sender_hex = hex::encode(&envelope.sender_key);
    tracing::debug!(from = %sender_hex, payload_len = envelope.payload.len(), "processing verified envelope");

    if is_blocked(state, pool, &sender_hex).await {
        tracing::debug!(from = %sender_hex, "dropping message from blocked user");
        return None;
    }

    let payload_bytes = decrypt_payload(state, app_handle, &sender_hex, &envelope.payload)?;

    let payload = match parse_payload(&payload_bytes) {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, from = %sender_hex, "failed to parse message payload");
            return None;
        }
    };

    if !matches!(
        payload,
        MessagePayload::FriendRequest { .. }
            | MessagePayload::FriendRequestReceived
            | MessagePayload::Unfriended
            | MessagePayload::UnfriendedAck
            | MessagePayload::FriendReject
            | MessagePayload::RelayEnvelope { .. }
            | MessagePayload::DmInvite { .. }
            | MessagePayload::GroupDmInvite { .. }
            | MessagePayload::WakeNotify { .. }
    ) && !state_helpers::is_friend(state, &sender_hex)
    {
        tracing::debug!(from = %sender_hex, "dropping message from non-friend");
        return None;
    }

    let ts: i64 = envelope.timestamp.try_into().unwrap_or(i64::MAX);
    Some(PreparedMessage {
        sender_hex,
        payload,
        timestamp: ts,
    })
}

/// Attempt Signal decryption; pass through if already valid JSON (plaintext).
///
/// Returns `None` on decrypt failure (after emitting a notification to the frontend)
/// or if no Signal manager is available for a non-JSON payload.
fn decrypt_payload(
    state: &Arc<AppState>,
    app_handle: &tauri::AppHandle,
    sender_hex: &str,
    raw_payload: &[u8],
) -> Option<Vec<u8>> {
    if serde_json::from_slice::<serde_json::Value>(raw_payload).is_ok() {
        return Some(raw_payload.to_vec());
    }

    let signal = state.signal_manager.lock();
    if let Some(handle) = signal.as_ref() {
        match handle.manager.decrypt(sender_hex, raw_payload) {
            Ok(pt) => Some(pt),
            Err(e) => {
                tracing::warn!(
                    error = %e, from = %sender_hex,
                    payload_len = raw_payload.len(),
                    "encrypted message could not be decrypted"
                );
                let display_name = state_helpers::friend_display_name(state, sender_hex);
                let from_label = display_name
                    .unwrap_or_else(|| format!("{}...", &sender_hex[..8.min(sender_hex.len())]));
                let _ = app_handle.emit(
                    "notification-event",
                    &crate::channels::NotificationEvent::SystemAlert {
                        title: "Message Decrypt Failed".to_string(),
                        body: format!(
                            "A message from {from_label} could not be decrypted. \
                             They may need to re-establish their session."
                        ),
                    },
                );
                None
            }
        }
    } else {
        tracing::warn!(from = %sender_hex, "received non-JSON payload but no signal manager");
        None
    }
}

/// Store a direct message in `SQLite` and emit `ChatEvent` to frontend.
fn handle_direct_message(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &DbPool,
    sender_hex: &str,
    body: &str,
    timestamp: i64,
) {
    // Store in SQLite (scoped to current identity)
    let owner_key = state_helpers::owner_key_or_default(state);
    let sender = sender_hex.to_string();
    let body_clone = body.to_string();
    db_fire(pool, "persist incoming message", move |conn| {
        crate::message_repo::insert_dm(
            conn,
            &owner_key,
            &sender,
            &sender,
            &body_clone,
            timestamp,
            false,
        )
    });

    // Update unread count
    {
        let mut friends = state.friends.write();
        if let Some(friend) = friends.get_mut(sender_hex) {
            friend.unread_count += 1;
        }
    }

    // Emit to frontend
    let event = ChatEvent::MessageReceived {
        from: sender_hex.to_string(),
        body: body.to_string(),
        decryption_failed: false,
        automod_blurred: false,
        timestamp: timestamp.cast_unsigned(),
        conversation_id: sender_hex.to_string(),
        server_message_id: None, // DMs have no message ID
        reply_to_id: None,
        sender_display_name: None, // DMs use friend list for name resolution
    };
    let _ = app_handle.emit("chat-event", &event);
}

/// Store a channel message in `SQLite` and emit `ChatEvent` to frontend.
fn handle_channel_message(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &DbPool,
    sender_hex: &str,
    channel_id: &str,
    body: &str,
    timestamp: i64,
) {
    let owner_key = state_helpers::owner_key_or_default(state);
    let sender = sender_hex.to_string();
    let ch_id = channel_id.to_string();
    let body_clone = body.to_string();
    db_fire(pool, "persist channel message", move |conn| {
        crate::message_repo::insert_channel_message(
            conn,
            &owner_key,
            &ch_id,
            &sender,
            &body_clone,
            timestamp,
            false,
            None,
        )
    });

    let event = ChatEvent::MessageReceived {
        from: sender_hex.to_string(),
        body: body.to_string(),
        decryption_failed: false,
        automod_blurred: false,
        timestamp: timestamp.cast_unsigned(),
        conversation_id: channel_id.to_string(),
        server_message_id: None, // P2P channel messages — ID assigned by sender
        reply_to_id: None,
        sender_display_name: None, // 1:1 channels use friend list for name resolution
    };
    let _ = app_handle.emit("chat-event", &event);
}

/// Process incoming friend request — just log receipt.
///
/// We do NOT establish a Signal session here. The session will be established
/// when we accept the request (we become the initiator), and the ephemeral key
/// is sent back in the `FriendAccept` so the requester can call `respond_to_session()`.
fn handle_friend_request(sender_hex: &str, prekey_bundle_bytes: &[u8]) {
    if prekey_bundle_bytes.is_empty() {
        tracing::warn!(from = %sender_hex, "friend request has empty prekey bundle");
    } else {
        tracing::info!(
            from = %sender_hex,
            prekey_len = prekey_bundle_bytes.len(),
            "received friend request — prekey bundle stored for later session establishment"
        );
    }
}

/// Process friend accept: establish *responder-side* Signal session.
///
/// The acceptor was the initiator (they called `establish_session`), so we are
/// the responder. We use the ephemeral key they sent us to derive a matching
/// shared secret via `respond_to_session()`.
fn handle_friend_accept(
    state: &Arc<AppState>,
    sender_hex: &str,
    prekey_bundle_bytes: &[u8],
    ephemeral_key: &[u8],
    signed_prekey_id: u32,
    one_time_prekey_id: Option<u32>,
) {
    if ephemeral_key.is_empty() {
        tracing::warn!(
            from = %sender_hex,
            "FriendAccept missing ephemeral key — no Signal session (legacy accept?)"
        );
        return;
    }

    // Extract their identity key from the PreKeyBundle
    let their_identity_key = match serde_json::from_slice::<rekindle_crypto::signal::PreKeyBundle>(
        prekey_bundle_bytes,
    ) {
        Ok(bundle) => bundle.identity_key,
        Err(e) => {
            tracing::warn!(from = %sender_hex, error = %e, "failed to parse PreKeyBundle from FriendAccept");
            return;
        }
    };

    let signal = state.signal_manager.lock();
    if let Some(handle) = signal.as_ref() {
        // Clear any stale session first (e.g., from a previous friendship that was removed)
        let _ = handle.manager.delete_session(sender_hex);
        match handle.manager.respond_to_session(
            sender_hex,
            &their_identity_key,
            ephemeral_key,
            signed_prekey_id,
            one_time_prekey_id,
        ) {
            Ok(()) => {
                tracing::info!(from = %sender_hex, "established responder Signal session from FriendAccept");
            }
            Err(e) => {
                tracing::warn!(from = %sender_hex, error = %e, "failed to establish responder Signal session");
            }
        }
    }
}

/// Handle a `FriendRequest` with profile key, route blob, and mailbox key exchange.
async fn handle_friend_request_full(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &DbPool,
    req: &IncomingFriendRequest<'_>,
) {
    handle_friend_request(req.sender_hex, req.prekey_bundle);

    // Cache the sender's route blob for immediate replies
    tracing::info!(
        from = %req.sender_hex,
        route_blob_len = req.route_blob.len(),
        route_count = req.route_blob.first().copied().unwrap_or(0),
        "handle_friend_request_full: received peer route blob"
    );
    if !req.route_blob.is_empty() {
        state_helpers::cache_peer_route(state, req.sender_hex, req.route_blob.to_vec());
    }

    // If sender is already in our friend list, check for cross-request auto-accept
    let existing_friendship_state =
        state_helpers::friend_field(state, req.sender_hex, |f| Some(f.friendship_state));

    if let Some(fs) = existing_friendship_state {
        if fs == crate::state::FriendshipState::PendingOut {
            // Cross-request: both parties want the friendship — auto-accept
            tracing::info!(from = %req.sender_hex, "cross-request detected — auto-accepting");
            auto_accept_cross_request(app_handle, state, pool, req).await;
            return;
        }
        if fs == crate::state::FriendshipState::Removing
            || fs == crate::state::FriendshipState::Accepted
        {
            // Removing: previous friendship being removed — clear stale state.
            // Accepted: peer removed us (their Unfriended was lost/delayed) and
            // re-added us. Following Briar's "re-add = new contact" pattern:
            // remove stale friendship state and treat as fresh incoming request.
            state.friends.write().remove(req.sender_hex);

            // Delete stale DB rows so future accept creates a clean entry
            crate::friend_repo::fire_delete_friend(state, pool, req.sender_hex);

            // Clean up any lingering pending request row
            delete_pending_request_row(state, pool, req.sender_hex);

            tracing::info!(
                from = %req.sender_hex,
                previous_state = ?fs,
                "received friend request from {} peer — treating as new request",
                if fs == crate::state::FriendshipState::Removing { "Removing" } else { "Accepted" }
            );
        } else {
            // PendingIn or other unexpected state — just update display name
            {
                let mut friends = state.friends.write();
                if let Some(friend) = friends.get_mut(req.sender_hex) {
                    friend.display_name = req.display_name.to_string();
                }
            }
            crate::friend_repo::fire_update_display_name(
                state,
                pool,
                req.sender_hex,
                req.display_name,
            );
            return;
        }
    }

    // Invite correlation: if this request carries an invite_id, check if cancelled
    if let Some(iid) = req.invite_id {
        let owner_key = state_helpers::owner_key_or_default(state);
        if crate::invite_helpers::is_invite_cancelled(pool, &owner_key, iid).await {
            tracing::info!(from = %req.sender_hex, %iid, "rejecting request for cancelled invite");
            let _ = send_friend_reject(state, pool, req.sender_hex).await;
            return;
        }
        crate::invite_helpers::mark_invite_responded(pool, &owner_key, iid, req.sender_hex);
    }

    // B5/P3.1 — persist BEFORE emit so the DB row exists by the time
    // chat-event reaches the frontend. Crash between emit-and-persist
    // (the prior db_fire spawn behavior) left a phantom request in memory
    // that vanished on restart and could never be accepted.
    if let Err(e) = persist_friend_request(state, pool, req).await {
        tracing::warn!(
            from = %req.sender_hex,
            error = %e,
            "failed to persist friend request — skipping event emit and ACK to avoid phantom UI state"
        );
        return;
    }
    let event = ChatEvent::FriendRequest {
        from: req.sender_hex.to_string(),
        display_name: req.display_name.to_string(),
        message: req.message.to_string(),
    };
    let _ = app_handle.emit("chat-event", &event);

    // B10/P3.4 — try the ACK send immediately; if it fails (peer offline,
    // route stale, app_message rejected), queue for retry through the
    // sync_service pending_messages loop. The previous `let _ = ...` swallow
    // meant the sender never learned we received their friend request and
    // kept retrying it forever from their side, eventually appearing
    // duplicate-spammy in the receiver's buddy list. The queue is bounded
    // (20 retries × 30s = 10 minutes per the existing sync_service drop
    // policy) so this can't loop forever.
    if let Err(e) = send_to_peer_raw(
        state,
        pool,
        req.sender_hex,
        &MessagePayload::FriendRequestReceived,
    )
    .await
    {
        tracing::info!(
            to = %req.sender_hex,
            error = %e,
            "FriendRequestReceived ACK send failed, queueing for sync_service retry"
        );
        if let Err(e) = build_and_queue_envelope(
            state,
            pool,
            req.sender_hex,
            &MessagePayload::FriendRequestReceived,
        )
        .await
        {
            tracing::warn!(
                to = %req.sender_hex,
                error = %e,
                "failed to queue FriendRequestReceived ACK for retry — sender may keep re-sending the request"
            );
        }
    }
}

/// Handle a `FriendAccept` with profile key, route blob, and mailbox key exchange.
async fn handle_friend_accept_full(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &DbPool,
    a: &IncomingFriendAccept<'_>,
) {
    // Guard: ignore FriendAccept if we are in the process of removing this friend
    let is_removing = state_helpers::friend_field(state, a.sender_hex, |f| {
        Some(matches!(
            f.friendship_state,
            crate::state::FriendshipState::Removing
        ))
    })
    .unwrap_or(false);
    if is_removing {
        tracing::info!(from = %a.sender_hex, "ignoring FriendAccept — friend is being removed");
        return;
    }

    handle_friend_accept(
        state,
        a.sender_hex,
        a.prekey_bundle,
        a.ephemeral_key,
        a.signed_prekey_id,
        a.one_time_prekey_id,
    );
    // Cache the acceptor's route blob
    if !a.route_blob.is_empty() {
        state_helpers::cache_peer_route(state, a.sender_hex, a.route_blob.clone());
    }
    // Store profile key, mailbox key, and transition friendship to Accepted
    {
        let mut friends = state.friends.write();
        if let Some(friend) = friends.get_mut(a.sender_hex) {
            if !a.profile_dht_key.is_empty() {
                friend.dht_record_key = Some(a.profile_dht_key.to_string());
            }
            if !a.mailbox_dht_key.is_empty() {
                friend.mailbox_dht_key = Some(a.mailbox_dht_key.to_string());
            }
            friend.friendship_state = crate::state::FriendshipState::Accepted;
        }
    }
    // Persist friendship_state transition to DB
    crate::friend_repo::fire_update_friendship_state(state, pool, a.sender_hex, "accepted");
    // Persist profile key to `SQLite`
    if !a.profile_dht_key.is_empty() {
        crate::friend_repo::fire_update_dht_record_key(
            state,
            pool,
            a.sender_hex,
            a.profile_dht_key,
        );
        // Start watching the friend's profile DHT record for presence
        if let Err(e) =
            super::presence_service::watch_friend(state, a.sender_hex, a.profile_dht_key).await
        {
            tracing::trace!(from = %a.sender_hex, error = %e, "failed to watch friend after accept");
        }
    }
    let display_name = state_helpers::friend_display_name(state, a.sender_hex)
        .unwrap_or_else(|| a.sender_hex.to_string());
    let event = ChatEvent::FriendRequestAccepted {
        from: a.sender_hex.to_string(),
        display_name,
    };
    let _ = app_handle.emit("chat-event", &event);
}

/// Handle a `ProfileKeyRotated` message from a friend.
async fn handle_profile_key_rotated(
    state: &Arc<AppState>,
    pool: &DbPool,
    sender_hex: &str,
    new_profile_dht_key: &str,
) {
    if !state_helpers::is_friend(state, sender_hex) {
        return;
    }
    // Unregister old DHT key
    let old_key = state_helpers::friend_dht_key(state, sender_hex);
    if let Some(ref old_key) = old_key {
        let mut dht_mgr = state.dht_manager.write();
        if let Some(mgr) = dht_mgr.as_mut() {
            mgr.unregister_friend_dht_key(old_key);
        }
    }
    // Update in-memory state
    {
        let mut friends = state.friends.write();
        if let Some(friend) = friends.get_mut(sender_hex) {
            friend.dht_record_key = Some(new_profile_dht_key.to_string());
        }
    }
    // Persist to `SQLite`
    crate::friend_repo::fire_update_dht_record_key(state, pool, sender_hex, new_profile_dht_key);
    // Re-watch the new profile DHT record for presence updates
    if let Err(e) =
        super::presence_service::watch_friend(state, sender_hex, new_profile_dht_key).await
    {
        tracing::warn!(from = %sender_hex, error = %e, "failed to watch new profile key");
    }
    tracing::info!(
        from = %sender_hex,
        new_key = %new_profile_dht_key,
        "friend rotated their profile DHT key"
    );
}

/// Persist an incoming friend request to `SQLite` for crash/restart recovery.
///
/// B5/P3.1 — synchronous persist (await the DB write) so the row exists
/// before the `ChatEvent::FriendRequest` event reaches the frontend. The
/// previous `db_fire` fire-and-forget spawn meant the event could land
/// in the buddy list while the row was still queued; a crash mid-window
/// left a phantom request in memory that vanished on restart and could
/// never be accepted. With awaited persist, the event is the trailing edge
/// of a durable write — the frontend never sees an entry whose backing
/// row doesn't exist. Returns `Err` so the caller can skip the event emit
/// when the persist itself failed (rather than emitting an event whose
/// row never landed).
async fn persist_friend_request(
    state: &Arc<AppState>,
    pool: &DbPool,
    req: &IncomingFriendRequest<'_>,
) -> Result<(), String> {
    let owner_key = state_helpers::owner_key_or_default(state);
    let pk = req.sender_hex.to_string();
    let dn = req.display_name.to_string();
    let msg = req.message.to_string();
    let pdk = req.profile_dht_key.to_string();
    let rb = req.route_blob.to_vec();
    let mdk = req.mailbox_dht_key.to_string();
    let pkb = req.prekey_bundle.to_vec();
    let iid = req.invite_id.map(str::to_string);
    let now = crate::db::timestamp_now();
    db_call(pool, move |conn| {
        conn.execute(
            "INSERT OR REPLACE INTO pending_friend_requests \
             (owner_key, public_key, display_name, message, received_at, profile_dht_key, route_blob, mailbox_dht_key, prekey_bundle, invite_id) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            rusqlite::params![owner_key, pk, dn, msg, now, pdk, rb, mdk, pkb, iid],
        )?;
        Ok(())
    })
    .await
}

/// Delete a `pending_friend_requests` row for a given peer.
///
/// Called during cross-request auto-accept, unfriend handling, and friend removal
/// to ensure stale rows don't block future `INSERT OR REPLACE`.
fn delete_pending_request_row(state: &Arc<AppState>, pool: &DbPool, peer_key: &str) {
    let owner_key = state_helpers::owner_key_or_default(state);
    let pk = peer_key.to_string();
    db_fire(pool, "delete pending request row", move |conn| {
        conn.execute(
            "DELETE FROM pending_friend_requests WHERE owner_key = ?1 AND public_key = ?2",
            rusqlite::params![owner_key, pk],
        )?;
        Ok(())
    });
}

/// Delete all `pending_messages` rows addressed to a given recipient.
///
/// Called when the peer ACKs our `Unfriended` message (no longer need retries)
/// or when a peer unfriends us (drop any queued messages to them).
pub(crate) fn delete_pending_messages_to_recipient(
    state: &Arc<AppState>,
    pool: &DbPool,
    recipient_key: &str,
) {
    let owner_key = state_helpers::owner_key_or_default(state);
    let rk = recipient_key.to_string();
    db_fire(pool, "delete pending messages to recipient", move |conn| {
        conn.execute(
            "DELETE FROM pending_messages WHERE owner_key = ?1 AND recipient_key = ?2",
            rusqlite::params![owner_key, rk],
        )?;
        Ok(())
    });
}

/// Handle an incoming `UnfriendedAck`: the peer confirms they processed our
/// `Unfriended` message. Clear any remaining retry queue entries for them.
fn handle_unfriended_ack(state: &Arc<AppState>, pool: &DbPool, sender_hex: &str) {
    delete_pending_messages_to_recipient(state, pool, sender_hex);
    tracing::info!(from = %sender_hex, "received UnfriendedAck — cleared pending messages");
}

/// Handle a `FriendReject` — if the rejected peer is in our `pending_out` list,
/// remove them. Otherwise, just emit the event.
fn handle_friend_reject(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &DbPool,
    sender_hex: &str,
) {
    let is_pending_out = state_helpers::friend_field(state, sender_hex, |f| {
        Some(f.friendship_state == crate::state::FriendshipState::PendingOut)
    })
    .unwrap_or(false);

    if is_pending_out {
        // Remove pending-out friend from DB and in-memory state
        crate::friend_repo::fire_delete_friend(state, pool, sender_hex);
        state.friends.write().remove(sender_hex);

        let _ = app_handle.emit(
            "chat-event",
            &ChatEvent::FriendRemoved {
                public_key: sender_hex.to_string(),
            },
        );
    }

    // Always emit the rejection notification
    let _ = app_handle.emit(
        "chat-event",
        &ChatEvent::FriendRequestRejected {
            from: sender_hex.to_string(),
        },
    );
}

/// Handle an incoming `Unfriended` message: the peer has removed us as a friend.
///
/// Removes the peer from our friends list (DB + in-memory), unregisters their
/// DHT presence key, updates our DHT friend list, and emits `FriendRemoved`
/// so the frontend updates.
async fn handle_unfriended(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &DbPool,
    sender_hex: &str,
) {
    // Only act if the sender is actually in our friends list
    let has_friend = state_helpers::friend_field(state, sender_hex, |f| {
        Some(!matches!(
            f.friendship_state,
            crate::state::FriendshipState::Removing
        ))
    })
    .unwrap_or(false);
    if !has_friend {
        tracing::debug!(from = %sender_hex, "ignoring Unfriended from non-friend");
        return;
    }

    // Remove from DB
    crate::friend_repo::fire_delete_friend(state, pool, sender_hex);

    // Clean up any pending request from this peer to prevent stale rows blocking future requests
    delete_pending_request_row(state, pool, sender_hex);

    // Drop any queued messages we were going to send them (they've unfriended us)
    delete_pending_messages_to_recipient(state, pool, sender_hex);

    // Send ACK back so the peer can clear their retry queue
    let _ = send_to_peer_raw(state, pool, sender_hex, &MessagePayload::UnfriendedAck).await;

    // Remove from in-memory state and unregister DHT key
    let dht_key = {
        let mut friends = state.friends.write();
        let removed = friends.remove(sender_hex);
        removed.and_then(|f| f.dht_record_key)
    };
    if let Some(ref dht_key) = dht_key {
        let mut dht_mgr = state.dht_manager.write();
        if let Some(mgr) = dht_mgr.as_mut() {
            mgr.unregister_friend_dht_key(dht_key);
        }
    }

    // Update our DHT friend list to reflect the removal
    if let Err(e) = push_friend_list_update(state).await {
        tracing::warn!(error = %e, "failed to update DHT friend list after peer unfriended us");
    }

    let _ = app_handle.emit(
        "chat-event",
        &ChatEvent::FriendRemoved {
            public_key: sender_hex.to_string(),
        },
    );

    tracing::info!(from = %sender_hex, "removed by peer (Unfriended)");
}

/// Auto-accept a cross-request: both parties sent friend requests to each other.
///
/// Transitions the local friend from `PendingOut` to `Accepted`, establishes
/// a Signal session, sends a `FriendAccept` back, and starts watching their DHT.
async fn auto_accept_cross_request(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &DbPool,
    req: &IncomingFriendRequest<'_>,
) {
    // 0. Clean up any lingering pending_friend_requests row so future requests start fresh
    delete_pending_request_row(state, pool, req.sender_hex);

    // 1. Transition local friend to Accepted + update keys
    {
        let mut friends = state.friends.write();
        if let Some(friend) = friends.get_mut(req.sender_hex) {
            friend.friendship_state = crate::state::FriendshipState::Accepted;
            friend.display_name = req.display_name.to_string();
            if !req.profile_dht_key.is_empty() {
                friend.dht_record_key = Some(req.profile_dht_key.to_string());
            }
            if !req.mailbox_dht_key.is_empty() {
                friend.mailbox_dht_key = Some(req.mailbox_dht_key.to_string());
            }
        }
    }
    crate::friend_repo::fire_update_friendship_state(state, pool, req.sender_hex, "accepted");
    crate::friend_repo::fire_update_display_name(state, pool, req.sender_hex, req.display_name);

    // Persist profile/mailbox keys
    if !req.profile_dht_key.is_empty() {
        crate::friend_repo::fire_update_dht_record_key(
            state,
            pool,
            req.sender_hex,
            req.profile_dht_key,
        );
    }
    if !req.mailbox_dht_key.is_empty() {
        crate::friend_repo::fire_update_mailbox_dht_key(
            state,
            pool,
            req.sender_hex,
            req.mailbox_dht_key,
        );
    }

    // 2. Establish Signal session from their prekey bundle
    // Clear any stale session first (e.g., from a previous friendship that was removed)
    let session_init = if req.prekey_bundle.is_empty() {
        None
    } else {
        let signal = state.signal_manager.lock();
        if let Some(handle) = signal.as_ref() {
            let _ = handle.manager.delete_session(req.sender_hex);
            if let Ok(bundle) =
                serde_json::from_slice::<rekindle_crypto::signal::PreKeyBundle>(req.prekey_bundle)
            {
                match handle.manager.establish_session(req.sender_hex, &bundle) {
                    Ok(info) => {
                        tracing::info!(peer = %req.sender_hex, "established Signal session on cross-request auto-accept");
                        Some(info)
                    }
                    Err(e) => {
                        tracing::warn!(peer = %req.sender_hex, error = %e, "failed to establish Signal session on cross-request");
                        None
                    }
                }
            } else {
                None
            }
        } else {
            None
        }
    };

    // 3. Send FriendAccept back
    super::message_service::send_friend_accept(state, pool, req.sender_hex, session_init)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "failed to send friend accept for cross-request");
        });

    // 4. Watch their DHT profile for presence
    if !req.profile_dht_key.is_empty() {
        if let Err(e) =
            super::presence_service::watch_friend(state, req.sender_hex, req.profile_dht_key).await
        {
            tracing::trace!(error = %e, "failed to watch friend DHT after cross-request accept");
        }
    }

    // 5. Emit accepted event
    let _ = app_handle.emit(
        "chat-event",
        &ChatEvent::FriendRequestAccepted {
            from: req.sender_hex.to_string(),
            display_name: req.display_name.to_string(),
        },
    );
}

/// Check if a sender is in the blocked users table.
async fn is_blocked(state: &Arc<AppState>, pool: &DbPool, sender_hex: &str) -> bool {
    let owner_key = state_helpers::owner_key_or_default(state);
    let pk = sender_hex.to_string();
    db_call_or_default(pool, move |conn| {
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM blocked_users WHERE owner_key = ?1 AND public_key = ?2",
            rusqlite::params![owner_key, pk],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    })
    .await
}

// ---------------------------------------------------------------------------
// Sending
// ---------------------------------------------------------------------------

/// Attempt to fetch a fresh route blob from a peer's DHT profile (subkey 6).
///
/// Called inline during send when no cached route exists or after a send failure,
/// to recover without waiting for the 30-second sync loop. Returns the route blob
/// bytes on success, or `None` if the peer has no DHT key, the node is detached,
/// or the DHT fetch fails.
async fn try_fetch_route_from_dht(state: &Arc<AppState>, peer_id: &str) -> Option<Vec<u8>> {
    let dht_key_str = state_helpers::friend_dht_key(state, peer_id)?;

    let record_key: veilid_core::RecordKey = dht_key_str.parse().ok()?;

    let (api, routing_context) = state_helpers::safe_api_and_routing_context(state)?;

    // Open (no-op if already open) and force-refresh subkey 6 (route blob)
    let _ = routing_context
        .open_dht_record(record_key.clone(), None)
        .await;
    let value_data = routing_context
        .get_dht_value(record_key, 6, true)
        .await
        .ok()??;
    let route_blob = value_data.data().to_vec();
    if route_blob.is_empty() {
        return None;
    }

    // Cache the fresh route blob
    {
        let mut dht_mgr = state.dht_manager.write();
        if let Some(mgr) = dht_mgr.as_mut() {
            mgr.manager.cache_route(&api, peer_id, route_blob.clone());
        }
    }

    tracing::debug!(peer = %peer_id, "fetched fresh route blob from DHT inline");
    Some(route_blob)
}

/// Try to fetch a fresh route from DHT and send the envelope immediately.
///
/// Used as an inline recovery path when no cached route exists or after a send
/// failure, avoiding the 30-second sync loop wait. Returns `true` if the send
/// succeeded, `false` if no route could be obtained or the send failed.
async fn try_inline_route_refresh_and_send(
    state: &Arc<AppState>,
    to: &str,
    envelope: &rekindle_protocol::messaging::envelope::MessageEnvelope,
) -> bool {
    let Some(fresh_blob) = try_fetch_route_from_dht(state, to).await else {
        return false;
    };

    let retry = state_helpers::safe_routing_context(state).and_then(|rc| {
        state_helpers::import_route_blob(state, &fresh_blob)
            .ok()
            .map(|rid| (rid, rc))
    });

    if let Some((rid, rc)) = retry {
        if send_envelope(&rc, rid, envelope).await.is_ok() {
            return true;
        }
    }

    false
}

/// Spawn a background `StatusRequest` fan-out (architecture §13.5) so
/// any relay friend that holds a fresh snapshot of `to` can update our
/// local cache. Best-effort — does not block the caller.
fn probe_relay_friends_for_status(state: &Arc<AppState>, pool: &DbPool, to: &str) {
    let state_clone = state.clone();
    let pool_clone = pool.clone();
    let target = to.to_string();
    tokio::spawn(async move {
        crate::services::relay::presence::probe_friends_for_status(
            &state_clone,
            &pool_clone,
            &target,
        )
        .await;
    });
}

/// Strand Relay last-resort fallback (architecture §13.3): when both the
/// cached route and inline-DHT-refresh fail, look up the recipient's
/// published relay pool (profile DHT subkey 8) and forward the
/// already-built envelope through a random non-dummy relay friend.
/// Strand Relay last-resort fallback (architecture §13.3): fetches the
/// peer's profile DHT subkey 8 (relay pool) and forwards through a
/// random non-dummy entry. The friend profile record itself is kept
/// warm by `sync_service::sync_friend_dht_subkeys`, which reads
/// subkeys 2/4/5/6 every 30 seconds — Veilid's per-record TTL covers
/// subkey 8 by association, so we don't need a dedicated keepalive.
async fn try_relay_fallback_send(
    state: &Arc<AppState>,
    to: &str,
    envelope: &rekindle_protocol::messaging::envelope::MessageEnvelope,
) -> bool {
    let Some(dht_key_str) = state_helpers::friend_dht_key(state, to) else {
        return false;
    };
    let Ok(record_key) = dht_key_str.parse::<veilid_core::RecordKey>() else {
        return false;
    };
    let Some(routing_context) = state_helpers::safe_routing_context(state) else {
        return false;
    };
    let _ = routing_context
        .open_dht_record(record_key.clone(), None)
        .await;
    let pool_body = match routing_context
        .get_dht_value(
            record_key,
            rekindle_protocol::dht::profile::SUBKEY_RELAY_POOL,
            true,
        )
        .await
    {
        Ok(Some(v)) => v.data().to_vec(),
        _ => return false,
    };
    let Ok(envelope_bytes) = serde_json::to_vec(envelope) else {
        return false;
    };
    crate::services::relay::send::send_via_relay(state, to, &pool_body, &envelope_bytes)
        .await
        .is_ok()
}

/// Build a `MessageEnvelope`, optionally encrypt with Signal, and send via Veilid.
///
/// If no route exists for the peer, the message is queued for retry by `sync_service`.
/// Ephemeral payloads (typing indicators) are never queued — a stale typing indicator
/// delivered minutes later is worse than no indicator.
async fn send_envelope_to_peer(
    state: &Arc<AppState>,
    pool: &DbPool,
    to: &str,
    payload: &MessagePayload,
    encrypt: bool,
) -> Result<(), String> {
    let is_ephemeral = matches!(payload, MessagePayload::TypingIndicator { .. });
    // Serialize the payload
    let payload_bytes =
        serde_json::to_vec(payload).map_err(|e| format!("serialize payload: {e}"))?;

    // B8/P3.3 — when encrypt is requested, refuse to fall back to plaintext.
    // The previous `_ => payload_bytes` arm silently sent the body in clear
    // when has_session returned false or errored, which an active attacker
    // who can corrupt your local Signal session state could trigger to
    // intercept the plaintext.
    //
    // Vulnerable-user safety stance: fail-closed. The caller's Result<(),
    // String> propagates to the Tauri IPC layer; the frontend toast shows
    // the specific reason and points the user at the explicit
    // "Re-establish secure session" path. No silent downgrade.
    let final_payload = if encrypt {
        let signal = state.signal_manager.lock();
        let handle = signal.as_ref().ok_or_else(|| {
            format!(
                "No Signal manager — cannot send encrypted message to {to}. \
                 Sign in to initialize Signal sessions."
            )
        })?;
        match handle.manager.has_session(to) {
            Ok(true) => handle
                .manager
                .encrypt(to, &payload_bytes)
                .map_err(|e| format!("Signal encrypt failed for {to}: {e}"))?,
            Ok(false) => {
                return Err(format!(
                    "No secure session with {to}. They haven't completed a Signal handshake yet — \
                     they need to accept your friend request before you can send encrypted messages. \
                     Verify their safety number out-of-band before resuming sensitive conversation."
                ));
            }
            Err(e) => {
                return Err(format!(
                    "Signal session check failed for {to}: {e}. \
                     The session may be corrupted; re-establish from Friend → Reset Secure Session \
                     after verifying their safety number out-of-band."
                ));
            }
        }
    } else {
        payload_bytes
    };

    // Build signed envelope
    let secret_key = {
        let sk = state.identity_secret.lock();
        *sk.as_ref().ok_or("signing key not initialized")?
    };

    let timestamp = rekindle_utils::timestamp_ms();

    let nonce = {
        let mut buf = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut buf);
        buf.to_vec()
    };

    let envelope = build_envelope_from_secret(&secret_key, timestamp, nonce, final_payload);

    // Look up the peer's cached route blob and import the RouteId via cache
    let route_id_and_rc = state_helpers::try_import_peer_route(state, to);

    let Some((route_id, routing_context)) = route_id_and_rc else {
        if is_ephemeral {
            tracing::debug!(to = %to, "no cached route for peer — dropping ephemeral message");
            return Ok(());
        }
        // Inline DHT route re-fetch before queuing — avoids 30s wait for sync loop
        if try_inline_route_refresh_and_send(state, to, &envelope).await {
            tracing::info!(to = %to, "message sent via veilid (after inline route refresh)");
            return Ok(());
        }
        // Strand Relay fallback (architecture §13.3): try a mutual friend's
        // published relay pool before giving up to the queue.
        if try_relay_fallback_send(state, to, &envelope).await {
            tracing::info!(to = %to, "message sent via strand relay");
            return Ok(());
        }
        // Architecture §13.5: ask our friends if any of them hold a
        // cached status (and a fresh route blob) for the target. Any
        // late-arriving StatusResponse will rehydrate the route cache
        // for the next send.
        probe_relay_friends_for_status(state, pool, to);
        tracing::debug!(to = %to, "no cached route for peer — queuing message for retry");
        let envelope_json =
            serde_json::to_string(&envelope).map_err(|e| format!("serialize envelope: {e}"))?;
        queue_pending_message(state, pool, to, &envelope_json).await?;
        return Ok(());
    };

    if let Err(e) = send_envelope(&routing_context, route_id, &envelope).await {
        // Invalidate the stale cached route so the next retry fetches fresh from DHT
        {
            let mut dht_mgr = state.dht_manager.write();
            if let Some(mgr) = dht_mgr.as_mut() {
                mgr.manager.invalidate_route_for_peer(to);
            }
        }
        if is_ephemeral {
            tracing::debug!(to = %to, error = %e, "send failed — dropping ephemeral message");
            return Ok(());
        }
        // Inline DHT route re-fetch before queuing — avoids 30s wait for sync loop
        if try_inline_route_refresh_and_send(state, to, &envelope).await {
            tracing::info!(to = %to, "message sent via veilid (after send failure + inline route refresh)");
            return Ok(());
        }
        if try_relay_fallback_send(state, to, &envelope).await {
            tracing::info!(to = %to, "message sent via strand relay (after send failure)");
            return Ok(());
        }
        tracing::warn!(to = %to, error = %e, "send failed — queuing for retry");
        let envelope_json =
            serde_json::to_string(&envelope).map_err(|e| format!("serialize envelope: {e}"))?;
        queue_pending_message(state, pool, to, &envelope_json).await?;
        return Ok(());
    }

    tracing::info!(to = %to, "message sent via veilid");
    Ok(())
}

/// Send a request-response payload to a peer via Veilid `app_call` and
/// return the deserialized reply envelope. Used for the DM accept/decline
/// handshake (architecture §27.1) and other cases where the caller needs
/// a guaranteed reply rather than queue-on-failure.
pub async fn send_to_peer_call(
    state: &Arc<AppState>,
    to: &str,
    payload: &MessagePayload,
) -> Result<MessagePayload, String> {
    let payload_bytes =
        serde_json::to_vec(payload).map_err(|e| format!("serialize payload: {e}"))?;
    let secret_key = {
        let sk = state.identity_secret.lock();
        *sk.as_ref().ok_or("signing key not initialized")?
    };
    let timestamp = rekindle_utils::timestamp_ms();
    let nonce = {
        let mut buf = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut buf);
        buf.to_vec()
    };
    let envelope = build_envelope_from_secret(&secret_key, timestamp, nonce, payload_bytes);

    let (route_id, routing_context) = state_helpers::try_import_peer_route(state, to)
        .ok_or_else(|| format!("no cached route for peer {to}"))?;

    let reply_bytes =
        rekindle_protocol::messaging::sender::send_call(&routing_context, route_id, &envelope)
            .await
            .map_err(|e| format!("app_call: {e}"))?;

    // Replies are raw `MessagePayload` JSON (the receiver shapes their
    // reply directly, not as a full signed envelope).
    serde_json::from_slice::<MessagePayload>(&reply_bytes)
        .map_err(|e| format!("decode reply payload: {e}"))
}

/// Send a direct message to a peer via the Veilid network.
pub async fn send_message(
    state: &Arc<AppState>,
    pool: &DbPool,
    to: &str,
    body: &str,
) -> Result<(), String> {
    let payload = MessagePayload::DirectMessage {
        body: body.to_string(),
        reply_to: None,
    };
    // Encrypt DMs when a Signal session exists
    send_envelope_to_peer(state, pool, to, &payload, true).await
}

/// Send a friend request to a peer via Veilid.
///
/// Includes our `PreKeyBundle` so the receiver can establish a Signal session.
/// Sent unencrypted (no session with the peer yet).
pub async fn send_friend_request(
    state: &Arc<AppState>,
    pool: &DbPool,
    to: &str,
    message: &str,
    invite_id: Option<&str>,
) -> Result<(), String> {
    let display_name = state_helpers::current_identity(state)
        .map_err(|_| "identity not set".to_string())?
        .display_name;

    let prekey_bundle = {
        let signal = state.signal_manager.lock();
        if let Some(handle) = signal.as_ref() {
            match handle.manager.generate_prekey_bundle(1, Some(1)) {
                Ok(bundle) => serde_json::to_vec(&bundle).unwrap_or_default(),
                Err(e) => {
                    tracing::warn!(error = %e, "failed to generate PreKeyBundle for friend request");
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        }
    };

    // Gather our profile and mailbox DHT keys + route blob for the invite payload
    let (profile_dht_key, route_blob, mailbox_dht_key) = {
        let node = state.node.read();
        let nh = node.as_ref().ok_or("node not initialized")?;
        (
            nh.profile_dht_key.clone().unwrap_or_default(),
            nh.route_blob.clone().unwrap_or_default(),
            nh.mailbox_dht_key.clone().unwrap_or_default(),
        )
    };

    tracing::info!(
        to = %to,
        route_blob_len = route_blob.len(),
        route_count = route_blob.first().copied().unwrap_or(0),
        "send_friend_request: our route blob info"
    );
    if route_blob.is_empty() {
        tracing::warn!(
            "sending friend request with empty route blob — peer will fetch from DHT profile"
        );
    }

    let payload = MessagePayload::FriendRequest {
        display_name,
        message: message.to_string(),
        prekey_bundle,
        profile_dht_key,
        route_blob,
        mailbox_dht_key,
        invite_id: invite_id.map(str::to_string),
    };
    // Friend requests are NOT encrypted (no session yet)
    send_envelope_to_peer(state, pool, to, &payload, false).await
}

/// Send a friend acceptance to a peer via Veilid.
///
/// Includes our `PreKeyBundle` and (if available) the `SessionInitInfo` from
/// `establish_session()` so the requester can call `respond_to_session()`.
pub async fn send_friend_accept(
    state: &Arc<AppState>,
    pool: &DbPool,
    to: &str,
    session_init: Option<rekindle_crypto::signal::SessionInitInfo>,
) -> Result<(), String> {
    let prekey_bundle = {
        let signal = state.signal_manager.lock();
        if let Some(handle) = signal.as_ref() {
            match handle.manager.generate_prekey_bundle(1, Some(1)) {
                Ok(bundle) => serde_json::to_vec(&bundle).unwrap_or_default(),
                Err(e) => {
                    tracing::warn!(error = %e, "failed to generate PreKeyBundle for friend accept");
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        }
    };

    // Gather our profile and mailbox DHT keys + route blob
    let (profile_dht_key, route_blob, mailbox_dht_key) = {
        let node = state.node.read();
        let nh = node.as_ref().ok_or("node not initialized")?;
        (
            nh.profile_dht_key.clone().unwrap_or_default(),
            nh.route_blob.clone().unwrap_or_default(),
            nh.mailbox_dht_key.clone().unwrap_or_default(),
        )
    };

    if route_blob.is_empty() {
        tracing::warn!(
            "sending friend accept with empty route blob — peer will fetch from DHT profile"
        );
    }

    let payload = MessagePayload::FriendAccept {
        prekey_bundle,
        profile_dht_key,
        route_blob,
        mailbox_dht_key,
        ephemeral_key: session_init
            .as_ref()
            .map(|s| s.ephemeral_public_key.clone())
            .unwrap_or_default(),
        signed_prekey_id: session_init.as_ref().map_or(1, |s| s.signed_prekey_id),
        one_time_prekey_id: session_init.as_ref().and_then(|s| s.one_time_prekey_id),
    };
    // Friend accepts are NOT encrypted (the requester may not have our session yet)
    send_envelope_to_peer(state, pool, to, &payload, false).await
}

/// Send a friend rejection to a peer via Veilid.
pub async fn send_friend_reject(
    state: &Arc<AppState>,
    pool: &DbPool,
    to: &str,
) -> Result<(), String> {
    let payload = MessagePayload::FriendReject;
    // Rejections are NOT encrypted
    send_envelope_to_peer(state, pool, to, &payload, false).await
}

/// Send a typing indicator to a peer.
pub async fn send_typing(
    state: &Arc<AppState>,
    pool: &DbPool,
    to: &str,
    typing: bool,
) -> Result<(), String> {
    let payload = MessagePayload::TypingIndicator { typing };
    // Typing indicators use encryption if session exists
    send_envelope_to_peer(state, pool, to, &payload, true).await
}

/// Send a raw (unencrypted) payload to a peer.
///
/// Used for protocol-level messages like `ProfileKeyRotated` that don't need
/// E2E encryption (the content isn't secret — it's a public DHT key).
pub async fn send_to_peer_raw(
    state: &Arc<AppState>,
    pool: &DbPool,
    to: &str,
    payload: &MessagePayload,
) -> Result<(), String> {
    send_envelope_to_peer(state, pool, to, payload, false).await
}

// ---------------------------------------------------------------------------
// Pending message queue
// ---------------------------------------------------------------------------

/// Build a signed `MessageEnvelope` for the given payload and queue it in
/// `pending_messages` for retry by `sync_service`.
///
/// Used by `friends.rs` to always-queue an `Unfriended` message regardless of
/// whether the initial `send_to_peer_raw` succeeded (Veilid `app_message` has
/// no delivery guarantee). The queued entry is cleared when the peer sends an
/// `UnfriendedAck`, or dropped after max retries (20 x 30s).
pub(crate) async fn build_and_queue_envelope(
    state: &Arc<AppState>,
    pool: &DbPool,
    to: &str,
    payload: &MessagePayload,
) -> Result<(), String> {
    let payload_bytes =
        serde_json::to_vec(payload).map_err(|e| format!("serialize payload: {e}"))?;

    let secret_key = {
        let sk = state.identity_secret.lock();
        *sk.as_ref().ok_or("signing key not initialized")?
    };

    let timestamp = rekindle_utils::timestamp_ms();

    let nonce = {
        let mut buf = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut buf);
        buf.to_vec()
    };

    let envelope = build_envelope_from_secret(&secret_key, timestamp, nonce, payload_bytes);
    let envelope_json =
        serde_json::to_string(&envelope).map_err(|e| format!("serialize envelope: {e}"))?;
    queue_pending_message(state, pool, to, &envelope_json).await
}

/// Insert a message into the `pending_messages` table for later retry.
async fn queue_pending_message(
    state: &Arc<AppState>,
    pool: &DbPool,
    recipient_key: &str,
    body: &str,
) -> Result<(), String> {
    let owner_key = state_helpers::owner_key_or_default(state);
    let recipient = recipient_key.to_string();
    let body = body.to_string();
    let now = crate::db::timestamp_now();
    db_call(pool, move |conn| {
        conn.execute(
            "INSERT INTO pending_messages (owner_key, recipient_key, body, created_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![owner_key, recipient, body, now],
        )?;
        Ok(())
    })
    .await
}

// ---------------------------------------------------------------------------
// DHT push helpers
// ---------------------------------------------------------------------------

/// Push a local change to DHT immediately (not waiting for periodic sync).
pub async fn push_profile_update(
    state: &Arc<AppState>,
    subkey: u32,
    value: Vec<u8>,
) -> Result<(), String> {
    let (profile_key, routing_context, owner_keypair) = {
        let node = state.node.read();
        let nh = node.as_ref().ok_or("node not initialized")?;
        let pk = nh.profile_dht_key.clone().ok_or("no profile DHT key")?;
        (
            pk,
            nh.routing_context.clone(),
            nh.profile_owner_keypair.clone(),
        )
    };

    let record_key: veilid_core::RecordKey = profile_key
        .parse()
        .map_err(|e| format!("invalid profile key: {e}"))?;

    // Ensure the record is open with write access before writing.
    // Re-opening an already-open record is a no-op in Veilid.
    let _ = routing_context
        .open_dht_record(record_key.clone(), owner_keypair)
        .await
        .map_err(|e| format!("failed to open profile record for push: {e}"))?;

    routing_context
        .set_dht_value(record_key, subkey, value, None)
        .await
        .map_err(|e| format!("failed to push profile update: {e}"))?;

    tracing::debug!(subkey, profile_key = %profile_key, "pushed profile update to DHT");
    Ok(())
}

/// Push the local friend list to our DHT friend list record.
///
/// Serializes the current friend public keys as a JSON array and writes
/// it to our friend list DHT record (subkey 0).
pub async fn push_friend_list_update(state: &Arc<AppState>) -> Result<(), String> {
    let (friend_list_key, routing_context, owner_keypair, friend_keys) = {
        let node = state.node.read();
        let nh = node.as_ref().ok_or("node not initialized")?;
        let flk = nh
            .friend_list_dht_key
            .clone()
            .ok_or("no friend list DHT key")?;
        let rc = nh.routing_context.clone();
        let kp = nh.friend_list_owner_keypair.clone();
        let friends = state.friends.read();
        let keys: Vec<String> = friends
            .iter()
            .filter(|(_, f)| !matches!(f.friendship_state, crate::state::FriendshipState::Removing))
            .map(|(k, _)| k.clone())
            .collect();
        (flk, rc, kp, keys)
    };

    let record_key: veilid_core::RecordKey = friend_list_key
        .parse()
        .map_err(|e| format!("invalid friend list key: {e}"))?;

    // Ensure the record is open with write access before writing.
    let _ = routing_context
        .open_dht_record(record_key.clone(), owner_keypair)
        .await
        .map_err(|e| format!("failed to open friend list record for push: {e}"))?;

    let value =
        serde_json::to_vec(&friend_keys).map_err(|e| format!("serialize friend list: {e}"))?;

    routing_context
        .set_dht_value(record_key, 0, value, None)
        .await
        .map_err(|e| format!("failed to push friend list update: {e}"))?;

    tracing::debug!(
        friend_list_key = %friend_list_key,
        count = friend_keys.len(),
        "pushed friend list update to DHT"
    );
    Ok(())
}
