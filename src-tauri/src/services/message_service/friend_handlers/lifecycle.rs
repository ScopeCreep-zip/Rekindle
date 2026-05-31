//! Friend-lifecycle handlers: profile-key rotation, friend-reject,
//! unfriend (peer-initiated removal + our ACK), the cleanup helpers
//! `delete_pending_request_row` / `delete_pending_messages_to_recipient`,
//! and the cross-request auto-accept path.

use std::sync::Arc;

use rekindle_protocol::messaging::envelope::MessagePayload;

use crate::channels::ChatEvent;
use crate::db::DbPool;
use crate::db_helpers::db_fire;
use crate::state::AppState;
use crate::state_helpers;

use super::IncomingFriendRequest;
use crate::services::message_service::{push_friend_list_update, send_to_peer_raw};

pub(crate) async fn handle_profile_key_rotated(
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
        crate::services::presence_service::watch_friend(state, sender_hex, new_profile_dht_key)
            .await
    {
        tracing::warn!(from = %sender_hex, error = %e, "failed to watch new profile key");
    }
    tracing::info!(
        from = %sender_hex,
        new_key = %new_profile_dht_key,
        "friend rotated their profile DHT key"
    );
}

/// Delete a `pending_friend_requests` row for a given peer.
///
/// Called during cross-request auto-accept, unfriend handling, and friend removal
/// to ensure stale rows don't block future `INSERT OR REPLACE`.
pub(super) fn delete_pending_request_row(state: &Arc<AppState>, pool: &DbPool, peer_key: &str) {
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
pub(crate) fn handle_unfriended_ack(state: &Arc<AppState>, pool: &DbPool, sender_hex: &str) {
    delete_pending_messages_to_recipient(state, pool, sender_hex);
    tracing::info!(from = %sender_hex, "received UnfriendedAck — cleared pending messages");
}

/// Handle a `FriendReject` — if the rejected peer is in our `pending_out` list,
/// remove them. Otherwise, just emit the event.
pub(crate) fn handle_friend_reject(
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

        crate::event_dispatch::emit_live(
            app_handle,
            "chat-event",
            &ChatEvent::FriendRemoved {
                public_key: sender_hex.to_string(),
            },
        );
    }

    // Always emit the rejection notification
    crate::event_dispatch::emit_live(
        app_handle,
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
pub(crate) async fn handle_unfriended(
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

    crate::event_dispatch::emit_live(
        app_handle,
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
pub(super) async fn auto_accept_cross_request(
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
    //
    // W16.10e (fix B variant) — same idempotency guard as the
    // accept_request path: skip establish_session if we already have a
    // working session AND the bundle's identity_key matches our trusted
    // record. Without this, network-duplicated cross-request handling
    // wipes the working session on the second arrival.
    let session_init = if req.prekey_bundle.is_empty() {
        None
    } else {
        let signal = state.signal_manager.read();
        if let Some(handle) = signal.as_ref() {
            if let Ok(bundle) =
                serde_json::from_slice::<rekindle_crypto::signal::PreKeyBundle>(req.prekey_bundle)
            {
                let already_established =
                    handle.manager.has_session(req.sender_hex).unwrap_or(false)
                        && handle
                            .manager
                            .is_trusted_identity(req.sender_hex, &bundle.identity_key)
                            .unwrap_or(false);
                if already_established {
                    tracing::info!(peer = %req.sender_hex,
                        "session already established for peer — skipping establish_session \
                         on cross-request (W16.10e idempotency)");
                    None
                } else {
                    match handle.manager.establish_session(req.sender_hex, &bundle) {
                        Ok(info) => {
                            tracing::info!(peer = %req.sender_hex, "established Signal session on cross-request auto-accept");
                            Some(info)
                        }
                        Err(e) => {
                            // W16.10d — was silent warn. Surface so user
                            // can act if cross-request handshake fails.
                            tracing::error!(peer = %req.sender_hex, error = %e,
                            "failed to establish Signal session on cross-request — peer's encrypted DMs will fail AEAD on us");
                            let peer_label =
                                state_helpers::friend_display_name(state, req.sender_hex)
                                    .unwrap_or_else(|| {
                                        format!(
                                            "{}…",
                                            &req.sender_hex[..16.min(req.sender_hex.len())]
                                        )
                                    });
                            crate::event_dispatch::emit_live(
                                app_handle,
                                "notification-event",
                                &crate::channels::NotificationEvent::SystemAlert {
                                    title: "Couldn't establish secure session".into(),
                                    body: format!(
                                    "Cross-request auto-accept with {peer_label} failed at the \
                                     Signal handshake: {e}. Click 'Reset Secure Session' from \
                                     their friend menu after verifying their safety number \
                                     out-of-band."
                                ),
                                },
                            );
                            None
                        }
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
    crate::services::message_service::send_friend_accept(state, pool, req.sender_hex, session_init)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "failed to send friend accept for cross-request");
        });

    // 4. Watch their DHT profile for presence
    if !req.profile_dht_key.is_empty() {
        if let Err(e) = crate::services::presence_service::watch_friend(
            state,
            req.sender_hex,
            req.profile_dht_key,
        )
        .await
        {
            tracing::trace!(error = %e, "failed to watch friend DHT after cross-request accept");
        }
    }

    // 5. Emit accepted event
    crate::event_dispatch::emit_live(
        app_handle,
        "chat-event",
        &ChatEvent::FriendRequestAccepted {
            from: req.sender_hex.to_string(),
            display_name: req.display_name.to_string(),
        },
    );
}
