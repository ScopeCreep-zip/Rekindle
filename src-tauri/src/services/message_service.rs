use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use rand::RngCore as _;
use rekindle_protocol::messaging::envelope::MessagePayload;
use rekindle_protocol::messaging::receiver::{parse_payload, process_incoming};
use rekindle_protocol::messaging::sender::{build_envelope_from_secret, send_envelope};
use tauri::Emitter;

use crate::channels::ChatEvent;
use crate::db::DbPool;
use crate::state::AppState;

/// Handle an incoming message from the Veilid network.
///
/// Flow: parse envelope → verify signature → decrypt if session exists →
/// parse payload → dispatch by type (DM, friend request, typing, etc.)
#[allow(clippy::too_many_lines)]
pub async fn handle_incoming_message(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &DbPool,
    raw_message: &[u8],
) {
    // Step 1: Parse and verify envelope signature
    let envelope = match process_incoming(raw_message) {
        Ok(env) => env,
        Err(e) => {
            tracing::error!(error = %e, "failed to parse/verify incoming message envelope");
            return;
        }
    };

    let sender_hex = hex::encode(&envelope.sender_key);
    tracing::debug!(from = %sender_hex, payload_len = envelope.payload.len(), "processing verified envelope");

    // Block list filtering
    if is_blocked(state, pool, &sender_hex).await {
        tracing::debug!(from = %sender_hex, "dropping message from blocked user");
        return;
    }

    // Step 2: Decrypt payload — try plaintext JSON first, then Signal decrypt.
    // This avoids mangling payloads that were sent unencrypted (friend requests,
    // accepts, messages sent before a session was established).
    let payload_bytes = if serde_json::from_slice::<serde_json::Value>(&envelope.payload).is_ok() {
        // Already valid JSON — use as-is (plaintext or unencrypted message)
        envelope.payload.clone()
    } else {
        // Not valid JSON — must be Signal-encrypted ciphertext
        let signal = state.signal_manager.lock();
        if let Some(handle) = signal.as_ref() {
            match handle.manager.decrypt(&sender_hex, &envelope.payload) {
                Ok(pt) => pt,
                Err(e) => {
                    tracing::warn!(
                        error = %e, from = %sender_hex,
                        payload_len = envelope.payload.len(),
                        "encrypted message could not be decrypted"
                    );
                    // Notify the frontend so the user knows a message was lost
                    let display_name = {
                        let friends = state.friends.read();
                        friends.get(&sender_hex).map(|f| f.display_name.clone())
                    };
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
                    return;
                }
            }
        } else {
            tracing::warn!(from = %sender_hex, "received non-JSON payload but no signal manager");
            return;
        }
    };

    // Step 3: Deserialize the payload into a structured MessagePayload
    let payload = match parse_payload(&payload_bytes) {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, from = %sender_hex, "failed to parse message payload");
            return;
        }
    };

    // Non-friend filtering: only protocol-level messages allowed from non-friends.
    // Unfriended/FriendReject must pass so delayed deliveries still work even if
    // the sender was already removed from our friends list by some other path.
    if !matches!(
        payload,
        MessagePayload::FriendRequest { .. }
            | MessagePayload::FriendRequestReceived
            | MessagePayload::Unfriended
            | MessagePayload::FriendReject
    ) && !state.friends.read().contains_key(&sender_hex)
    {
        tracing::debug!(from = %sender_hex, "dropping message from non-friend");
        return;
    }

    // Step 4: Dispatch by payload type
    let ts: i64 = envelope.timestamp.try_into().unwrap_or(i64::MAX);
    match payload {
        MessagePayload::DirectMessage { body, .. } => {
            handle_direct_message(app_handle, state, pool, &sender_hex, &body, ts).await;
        }
        MessagePayload::ChannelMessage { channel_id, body, .. } => {
            handle_channel_message(app_handle, state, pool, &sender_hex, &channel_id, &body, ts).await;
        }
        MessagePayload::TypingIndicator { typing } => {
            let _ = app_handle.emit("chat-event", &ChatEvent::TypingIndicator { from: sender_hex, typing });
        }
        MessagePayload::FriendRequest { display_name, message, prekey_bundle, profile_dht_key, route_blob, mailbox_dht_key } => {
            handle_friend_request_full(
                app_handle, state, pool, &sender_hex, &display_name,
                &message, &prekey_bundle, &profile_dht_key, &route_blob, &mailbox_dht_key,
            ).await;
        }
        MessagePayload::FriendAccept { prekey_bundle, profile_dht_key, route_blob, mailbox_dht_key, ephemeral_key, signed_prekey_id, one_time_prekey_id } => {
            handle_friend_accept_full(
                app_handle, state, pool, &sender_hex, &prekey_bundle,
                &profile_dht_key, route_blob, &mailbox_dht_key,
                &ephemeral_key, signed_prekey_id, one_time_prekey_id,
            ).await;
        }
        MessagePayload::FriendReject => {
            handle_friend_reject(app_handle, state, pool, &sender_hex).await;
        }
        MessagePayload::FriendRequestReceived => {
            let _ = app_handle.emit("chat-event", &ChatEvent::FriendRequestDelivered { to: sender_hex });
        }
        MessagePayload::ProfileKeyRotated { new_profile_dht_key } => {
            handle_profile_key_rotated(state, pool, &sender_hex, &new_profile_dht_key).await;
        }
        MessagePayload::PresenceUpdate { .. } => {}
        MessagePayload::Unfriended => {
            handle_unfriended(app_handle, state, pool, &sender_hex).await;
        }
    }
}

/// Store a direct message in `SQLite` and emit `ChatEvent` to frontend.
async fn handle_direct_message(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &DbPool,
    sender_hex: &str,
    body: &str,
    timestamp: i64,
) {

    // Store in SQLite (scoped to current identity)
    let owner_key = state
        .identity
        .read()
        .as_ref()
        .map(|id| id.public_key.clone())
        .unwrap_or_default();
    let pool_clone = pool.clone();
    let sender = sender_hex.to_string();
    let body_clone = body.to_string();
    if let Err(e) = tokio::task::spawn_blocking(move || {
        let conn = pool_clone.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO messages (owner_key, conversation_id, conversation_type, sender_key, body, timestamp, is_read) \
             VALUES (?, ?, 'dm', ?, ?, ?, 0)",
            rusqlite::params![owner_key, sender, sender, body_clone, timestamp],
        )
        .map_err(|e| e.to_string())
    })
    .await
    .unwrap_or_else(|e| Err(e.to_string()))
    {
        tracing::error!(error = %e, "failed to persist incoming message");
    }

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
        timestamp: timestamp.cast_unsigned(),
        conversation_id: sender_hex.to_string(),
    };
    let _ = app_handle.emit("chat-event", &event);
}

/// Store a channel message in `SQLite` and emit `ChatEvent` to frontend.
async fn handle_channel_message(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &DbPool,
    sender_hex: &str,
    channel_id: &str,
    body: &str,
    timestamp: i64,
) {

    let owner_key = state
        .identity
        .read()
        .as_ref()
        .map(|id| id.public_key.clone())
        .unwrap_or_default();
    let pool_clone = pool.clone();
    let sender = sender_hex.to_string();
    let ch_id = channel_id.to_string();
    let body_clone = body.to_string();
    if let Err(e) = tokio::task::spawn_blocking(move || {
        let conn = pool_clone.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO messages (owner_key, conversation_id, conversation_type, sender_key, body, timestamp, is_read) \
             VALUES (?, ?, 'channel', ?, ?, ?, 0)",
            rusqlite::params![owner_key, ch_id, sender, body_clone, timestamp],
        )
        .map_err(|e| e.to_string())
    })
    .await
    .unwrap_or_else(|e| Err(e.to_string()))
    {
        tracing::error!(error = %e, "failed to persist channel message");
    }

    let event = ChatEvent::MessageReceived {
        from: sender_hex.to_string(),
        body: body.to_string(),
        timestamp: timestamp.cast_unsigned(),
        conversation_id: channel_id.to_string(),
    };
    let _ = app_handle.emit("chat-event", &event);
}

/// Process incoming friend request — just log receipt.
///
/// We do NOT establish a Signal session here. The session will be established
/// when we accept the request (we become the initiator), and the ephemeral key
/// is sent back in the `FriendAccept` so the requester can call `respond_to_session()`.
fn handle_friend_request(
    sender_hex: &str,
    prekey_bundle_bytes: &[u8],
) {
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
    let their_identity_key = match serde_json::from_slice::<rekindle_crypto::signal::PreKeyBundle>(prekey_bundle_bytes) {
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
            Ok(()) => tracing::info!(from = %sender_hex, "established responder Signal session from FriendAccept"),
            Err(e) => tracing::warn!(from = %sender_hex, error = %e, "failed to establish responder Signal session"),
        }
    }
}

/// Handle a `FriendRequest` with profile key, route blob, and mailbox key exchange.
#[allow(clippy::too_many_arguments)]
async fn handle_friend_request_full(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &DbPool,
    sender_hex: &str,
    display_name: &str,
    message: &str,
    prekey_bundle: &[u8],
    profile_dht_key: &str,
    route_blob: &[u8],
    mailbox_dht_key: &str,
) {
    handle_friend_request(sender_hex, prekey_bundle);

    // Cache the sender's route blob for immediate replies
    if !route_blob.is_empty() {
        let api = {
            let node = state.node.read();
            node.as_ref().map(|nh| nh.api.clone())
        };
        if let Some(api) = api {
            let mut dht_mgr = state.dht_manager.write();
            if let Some(mgr) = dht_mgr.as_mut() {
                mgr.manager.cache_route(&api, sender_hex, route_blob.to_vec());
            }
        }
    }

    // If sender is already in our friend list, check for cross-request auto-accept
    let existing_friendship_state = {
        let friends = state.friends.read();
        friends.get(sender_hex).map(|f| f.friendship_state)
    };

    if let Some(fs) = existing_friendship_state {
        if fs == crate::state::FriendshipState::PendingOut {
            // Cross-request: both parties want the friendship — auto-accept
            tracing::info!(from = %sender_hex, "cross-request detected — auto-accepting");
            auto_accept_cross_request(
                app_handle, state, pool, sender_hex, display_name,
                prekey_bundle, profile_dht_key, route_blob, mailbox_dht_key,
            ).await;
            return;
        }
        if fs == crate::state::FriendshipState::Removing {
            // Previous friendship being removed — clear stale state and treat
            // as a fresh incoming request (fall through to persist_friend_request).
            state.friends.write().remove(sender_hex);
            tracing::info!(from = %sender_hex, "received friend request from Removing peer — treating as new request");
        } else {
            // Already accepted friend — just update display name
            {
                let mut friends = state.friends.write();
                if let Some(friend) = friends.get_mut(sender_hex) {
                    friend.display_name = display_name.to_string();
                }
            }
            update_friend_display_name(state, pool, sender_hex, display_name).await;
            return;
        }
    }

    persist_friend_request(
        state, pool, sender_hex, display_name, message,
        profile_dht_key, route_blob, mailbox_dht_key, prekey_bundle,
    )
    .await;
    let event = ChatEvent::FriendRequest {
        from: sender_hex.to_string(),
        display_name: display_name.to_string(),
        message: message.to_string(),
    };
    let _ = app_handle.emit("chat-event", &event);

    // Send delivery ACK back to the requester
    let _ = send_to_peer_raw(state, pool, sender_hex, &MessagePayload::FriendRequestReceived).await;
}

/// Handle a `FriendAccept` with profile key, route blob, and mailbox key exchange.
#[allow(clippy::too_many_arguments)]
async fn handle_friend_accept_full(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &DbPool,
    sender_hex: &str,
    prekey_bundle: &[u8],
    profile_dht_key: &str,
    route_blob: Vec<u8>,
    mailbox_dht_key: &str,
    ephemeral_key: &[u8],
    signed_prekey_id: u32,
    one_time_prekey_id: Option<u32>,
) {
    handle_friend_accept(state, sender_hex, prekey_bundle, ephemeral_key, signed_prekey_id, one_time_prekey_id);
    // Cache the acceptor's route blob
    if !route_blob.is_empty() {
        let api = {
            let node = state.node.read();
            node.as_ref().map(|nh| nh.api.clone())
        };
        if let Some(api) = api {
            let mut dht_mgr = state.dht_manager.write();
            if let Some(mgr) = dht_mgr.as_mut() {
                mgr.manager.cache_route(&api, sender_hex, route_blob);
            }
        }
    }
    // Store profile key, mailbox key, and transition friendship to Accepted
    {
        let mut friends = state.friends.write();
        if let Some(friend) = friends.get_mut(sender_hex) {
            if !profile_dht_key.is_empty() {
                friend.dht_record_key = Some(profile_dht_key.to_string());
            }
            if !mailbox_dht_key.is_empty() {
                friend.mailbox_dht_key = Some(mailbox_dht_key.to_string());
            }
            friend.friendship_state = crate::state::FriendshipState::Accepted;
        }
    }
    // Persist friendship_state transition to DB
    persist_friendship_state(state, pool, sender_hex, "accepted").await;
    // Persist profile key to `SQLite`
    if !profile_dht_key.is_empty() {
        persist_friend_dht_key(state, pool, sender_hex, profile_dht_key).await;
        // Start watching the friend's profile DHT record for presence
        if let Err(e) =
            super::presence_service::watch_friend(state, sender_hex, profile_dht_key).await
        {
            tracing::trace!(from = %sender_hex, error = %e, "failed to watch friend after accept");
        }
    }
    let display_name = {
        let friends = state.friends.read();
        friends
            .get(sender_hex)
            .map_or_else(|| sender_hex.to_string(), |f| f.display_name.clone())
    };
    let event = ChatEvent::FriendRequestAccepted {
        from: sender_hex.to_string(),
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
    let is_friend = state.friends.read().contains_key(sender_hex);
    if !is_friend {
        return;
    }
    // Unregister old DHT key
    let old_key = {
        let friends = state.friends.read();
        friends
            .get(sender_hex)
            .and_then(|f| f.dht_record_key.clone())
    };
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
    persist_friend_dht_key(state, pool, sender_hex, new_profile_dht_key).await;
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
#[allow(clippy::too_many_arguments)]
async fn persist_friend_request(
    state: &Arc<AppState>,
    pool: &DbPool,
    sender_hex: &str,
    display_name: &str,
    message: &str,
    profile_dht_key: &str,
    route_blob: &[u8],
    mailbox_dht_key: &str,
    prekey_bundle: &[u8],
) {
    let owner_key = state
        .identity
        .read()
        .as_ref()
        .map(|id| id.public_key.clone())
        .unwrap_or_default();
    let pool = pool.clone();
    let pk = sender_hex.to_string();
    let dn = display_name.to_string();
    let msg = message.to_string();
    let pdk = profile_dht_key.to_string();
    let rb = route_blob.to_vec();
    let mdk = mailbox_dht_key.to_string();
    let pkb = prekey_bundle.to_vec();
    let now = crate::db::timestamp_now();
    if let Err(e) = tokio::task::spawn_blocking(move || {
        let conn = pool.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT OR IGNORE INTO pending_friend_requests \
             (owner_key, public_key, display_name, message, received_at, profile_dht_key, route_blob, mailbox_dht_key, prekey_bundle) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![owner_key, pk, dn, msg, now, pdk, rb, mdk, pkb],
        )
        .map_err(|e| e.to_string())
    })
    .await
    .unwrap_or_else(|e| Err(e.to_string()))
    {
        tracing::error!(error = %e, "failed to persist incoming friend request");
    }
}

/// Persist a friend's profile DHT key to `SQLite`.
async fn persist_friend_dht_key(
    state: &Arc<AppState>,
    pool: &DbPool,
    friend_key: &str,
    profile_dht_key: &str,
) {
    let owner_key = state
        .identity
        .read()
        .as_ref()
        .map(|id| id.public_key.clone())
        .unwrap_or_default();
    let pool = pool.clone();
    let fk = friend_key.to_string();
    let pdk = profile_dht_key.to_string();
    if let Err(e) = tokio::task::spawn_blocking(move || {
        let conn = pool.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE friends SET dht_record_key = ?1 WHERE owner_key = ?2 AND public_key = ?3",
            rusqlite::params![pdk, owner_key, fk],
        )
        .map_err(|e| e.to_string())
    })
    .await
    .unwrap_or_else(|e| Err(e.to_string()))
    {
        tracing::error!(error = %e, "failed to persist friend DHT key");
    }
}

/// Update a friend's display name in `SQLite`.
async fn update_friend_display_name(
    state: &Arc<AppState>,
    pool: &DbPool,
    public_key: &str,
    display_name: &str,
) {
    let owner_key = state
        .identity
        .read()
        .as_ref()
        .map(|id| id.public_key.clone())
        .unwrap_or_default();
    let pool = pool.clone();
    let pk = public_key.to_string();
    let dn = display_name.to_string();
    let _ = tokio::task::spawn_blocking(move || {
        let conn = pool.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE friends SET display_name = ?1 WHERE owner_key = ?2 AND public_key = ?3",
            rusqlite::params![dn, owner_key, pk],
        )
        .map_err(|e| format!("update friend display name: {e}"))?;
        Ok::<(), String>(())
    })
    .await;
}

/// Handle a `FriendReject` — if the rejected peer is in our `pending_out` list,
/// remove them. Otherwise, just emit the event.
async fn handle_friend_reject(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &DbPool,
    sender_hex: &str,
) {
    let is_pending_out = state
        .friends
        .read()
        .get(sender_hex)
        .is_some_and(|f| f.friendship_state == crate::state::FriendshipState::PendingOut);

    if is_pending_out {
        // Remove pending-out friend from DB and in-memory state
        let owner_key = state
            .identity
            .read()
            .as_ref()
            .map(|id| id.public_key.clone())
            .unwrap_or_default();
        let pool_clone = pool.clone();
        let pk = sender_hex.to_string();
        let ok = owner_key;
        let _ = tokio::task::spawn_blocking(move || {
            let conn = pool_clone.lock().map_err(|e| e.to_string())?;
            conn.execute(
                "DELETE FROM friends WHERE owner_key = ?1 AND public_key = ?2",
                rusqlite::params![ok, pk],
            )
            .map_err(|e| format!("delete rejected friend: {e}"))?;
            Ok::<(), String>(())
        })
        .await;
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
    let has_friend = {
        let friends = state.friends.read();
        friends.get(sender_hex).is_some_and(|f| {
            !matches!(f.friendship_state, crate::state::FriendshipState::Removing)
        })
    };
    if !has_friend {
        tracing::debug!(from = %sender_hex, "ignoring Unfriended from non-friend");
        return;
    }

    // Remove from DB
    let owner_key = state
        .identity
        .read()
        .as_ref()
        .map(|id| id.public_key.clone())
        .unwrap_or_default();
    let pool_clone = pool.clone();
    let pk = sender_hex.to_string();
    let ok = owner_key;
    let _ = tokio::task::spawn_blocking(move || {
        let conn = pool_clone.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM friends WHERE owner_key = ?1 AND public_key = ?2",
            rusqlite::params![ok, pk],
        )
        .map_err(|e| format!("delete unfriended peer: {e}"))?;
        Ok::<(), String>(())
    })
    .await;

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
#[allow(clippy::too_many_arguments)]
async fn auto_accept_cross_request(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &DbPool,
    sender_hex: &str,
    display_name: &str,
    prekey_bundle: &[u8],
    profile_dht_key: &str,
    _route_blob: &[u8],
    mailbox_dht_key: &str,
) {
    // 1. Transition local friend to Accepted + update keys
    {
        let mut friends = state.friends.write();
        if let Some(friend) = friends.get_mut(sender_hex) {
            friend.friendship_state = crate::state::FriendshipState::Accepted;
            friend.display_name = display_name.to_string();
            if !profile_dht_key.is_empty() {
                friend.dht_record_key = Some(profile_dht_key.to_string());
            }
            if !mailbox_dht_key.is_empty() {
                friend.mailbox_dht_key = Some(mailbox_dht_key.to_string());
            }
        }
    }
    persist_friendship_state(state, pool, sender_hex, "accepted").await;
    update_friend_display_name(state, pool, sender_hex, display_name).await;

    // Persist profile/mailbox keys
    if !profile_dht_key.is_empty() {
        persist_friend_dht_key(state, pool, sender_hex, profile_dht_key).await;
    }
    if !mailbox_dht_key.is_empty() {
        persist_friend_mailbox_key(state, pool, sender_hex, mailbox_dht_key).await;
    }

    // 2. Establish Signal session from their prekey bundle
    // Clear any stale session first (e.g., from a previous friendship that was removed)
    let session_init = if prekey_bundle.is_empty() {
        None
    } else {
        let signal = state.signal_manager.lock();
        if let Some(handle) = signal.as_ref() {
            let _ = handle.manager.delete_session(sender_hex);
            if let Ok(bundle) = serde_json::from_slice::<rekindle_crypto::signal::PreKeyBundle>(prekey_bundle) {
                match handle.manager.establish_session(sender_hex, &bundle) {
                    Ok(info) => {
                        tracing::info!(peer = %sender_hex, "established Signal session on cross-request auto-accept");
                        Some(info)
                    }
                    Err(e) => {
                        tracing::warn!(peer = %sender_hex, error = %e, "failed to establish Signal session on cross-request");
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
    super::message_service::send_friend_accept(state, pool, sender_hex, session_init)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "failed to send friend accept for cross-request");
        });

    // 4. Watch their DHT profile for presence
    if !profile_dht_key.is_empty() {
        if let Err(e) =
            super::presence_service::watch_friend(state, sender_hex, profile_dht_key).await
        {
            tracing::trace!(error = %e, "failed to watch friend DHT after cross-request accept");
        }
    }

    // 5. Emit accepted event
    let _ = app_handle.emit(
        "chat-event",
        &ChatEvent::FriendRequestAccepted {
            from: sender_hex.to_string(),
            display_name: display_name.to_string(),
        },
    );
}

/// Persist the `friendship_state` column to `SQLite` for a friend.
async fn persist_friendship_state(
    state: &Arc<AppState>,
    pool: &DbPool,
    friend_key: &str,
    friendship_state: &str,
) {
    let owner_key = state
        .identity
        .read()
        .as_ref()
        .map(|id| id.public_key.clone())
        .unwrap_or_default();
    let pool = pool.clone();
    let fk = friend_key.to_string();
    let fs = friendship_state.to_string();
    let _ = tokio::task::spawn_blocking(move || {
        let conn = pool.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE friends SET friendship_state = ?1 WHERE owner_key = ?2 AND public_key = ?3",
            rusqlite::params![fs, owner_key, fk],
        )
        .map_err(|e| format!("update friendship_state: {e}"))?;
        Ok::<(), String>(())
    })
    .await;
}

/// Persist a friend's mailbox DHT key to `SQLite`.
async fn persist_friend_mailbox_key(
    state: &Arc<AppState>,
    pool: &DbPool,
    friend_key: &str,
    mailbox_dht_key: &str,
) {
    let owner_key = state
        .identity
        .read()
        .as_ref()
        .map(|id| id.public_key.clone())
        .unwrap_or_default();
    let pool = pool.clone();
    let fk = friend_key.to_string();
    let mdk = mailbox_dht_key.to_string();
    let _ = tokio::task::spawn_blocking(move || {
        let conn = pool.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE friends SET mailbox_dht_key = ?1 WHERE owner_key = ?2 AND public_key = ?3",
            rusqlite::params![mdk, owner_key, fk],
        )
        .map_err(|e| format!("update friend mailbox key: {e}"))?;
        Ok::<(), String>(())
    })
    .await;
}

/// Check if a sender is in the blocked users table.
async fn is_blocked(state: &Arc<AppState>, pool: &DbPool, sender_hex: &str) -> bool {
    let owner_key = state
        .identity
        .read()
        .as_ref()
        .map(|id| id.public_key.clone())
        .unwrap_or_default();
    let pool = pool.clone();
    let pk = sender_hex.to_string();
    tokio::task::spawn_blocking(move || {
        let conn = pool.lock().map_err(|e| e.to_string())?;
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM blocked_users WHERE owner_key = ?1 AND public_key = ?2",
                rusqlite::params![owner_key, pk],
                |row| row.get(0),
            )
            .unwrap_or(0);
        Ok::<bool, String>(count > 0)
    })
    .await
    .unwrap_or(Ok(false))
    .unwrap_or(false)
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
async fn try_fetch_route_from_dht(
    state: &Arc<AppState>,
    peer_id: &str,
) -> Option<Vec<u8>> {
    let dht_key_str = {
        let friends = state.friends.read();
        friends.get(peer_id).and_then(|f| f.dht_record_key.clone())
    }?;

    let record_key: veilid_core::RecordKey = dht_key_str.parse().ok()?;

    let (routing_context, api) = {
        let node = state.node.read();
        let nh = node.as_ref()?;
        if !nh.is_attached {
            return None;
        }
        (nh.routing_context.clone(), nh.api.clone())
    };

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

    let retry = {
        let node = state.node.read();
        node.as_ref().and_then(|nh| {
            let api = nh.api.clone();
            let rc = nh.routing_context.clone();
            let mut dht_mgr = state.dht_manager.write();
            dht_mgr.as_mut().and_then(|mgr| {
                mgr.manager
                    .get_or_import_route(&api, &fresh_blob)
                    .ok()
                    .map(|rid| (rid, rc))
            })
        })
    };

    if let Some((rid, rc)) = retry {
        if send_envelope(&rc, rid, envelope).await.is_ok() {
            return true;
        }
    }

    false
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

    // Optionally encrypt with Signal
    let final_payload = if encrypt {
        let signal = state.signal_manager.lock();
        if let Some(handle) = signal.as_ref() {
            match handle.manager.has_session(to) {
                Ok(true) => handle
                    .manager
                    .encrypt(to, &payload_bytes)
                    .map_err(|e| format!("Signal encrypt: {e}"))?,
                _ => payload_bytes,
            }
        } else {
            payload_bytes
        }
    } else {
        payload_bytes
    };

    // Build signed envelope
    let secret_key = {
        let sk = state.identity_secret.lock();
        *sk.as_ref().ok_or("signing key not initialized")?
    };

    let timestamp = u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(u64::MAX);

    let nonce = {
        let mut buf = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut buf);
        buf.to_vec()
    };

    let envelope = build_envelope_from_secret(&secret_key, timestamp, nonce, final_payload);

    // Look up the peer's cached route blob and import the RouteId via cache
    let route_id_and_rc = {
        let node = state.node.read();
        let nh = node.as_ref().ok_or("node not initialized")?;
        let api = nh.api.clone();
        let rc = nh.routing_context.clone();

        let mut dht_mgr = state.dht_manager.write();
        let mgr = dht_mgr.as_mut().ok_or("DHT manager not initialized")?;

        match mgr.manager.get_cached_route(to).cloned() {
            Some(blob) => {
                match mgr.manager.get_or_import_route(&api, &blob) {
                    Ok(route_id) => Some((route_id, rc)),
                    Err(e) => {
                        tracing::warn!(
                            to = %to, error = %e, blob_len = blob.len(),
                            "route import failed — invalidating and queuing for retry"
                        );
                        mgr.manager.invalidate_route_for_peer(to);
                        None
                    }
                }
            }
            None => None,
        }
    };

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
        tracing::warn!(to = %to, error = %e, "send failed — queuing for retry");
        let envelope_json =
            serde_json::to_string(&envelope).map_err(|e| format!("serialize envelope: {e}"))?;
        queue_pending_message(state, pool, to, &envelope_json).await?;
        return Ok(());
    }

    tracing::info!(to = %to, "message sent via veilid");
    Ok(())
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
) -> Result<(), String> {
    let (display_name, prekey_bundle) = {
        let identity = state.identity.read();
        let id = identity.as_ref().ok_or("identity not set")?;
        let display_name = id.display_name.clone();

        let signal = state.signal_manager.lock();
        let bundle_bytes = if let Some(handle) = signal.as_ref() {
            match handle.manager.generate_prekey_bundle(1, Some(1)) {
                Ok(bundle) => serde_json::to_vec(&bundle).unwrap_or_default(),
                Err(e) => {
                    tracing::warn!(error = %e, "failed to generate PreKeyBundle for friend request");
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };
        (display_name, bundle_bytes)
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

    if route_blob.is_empty() {
        tracing::warn!("sending friend request with empty route blob — peer will fetch from DHT profile");
    }

    let payload = MessagePayload::FriendRequest {
        display_name,
        message: message.to_string(),
        prekey_bundle,
        profile_dht_key,
        route_blob,
        mailbox_dht_key,
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
        tracing::warn!("sending friend accept with empty route blob — peer will fetch from DHT profile");
    }

    let payload = MessagePayload::FriendAccept {
        prekey_bundle,
        profile_dht_key,
        route_blob,
        mailbox_dht_key,
        ephemeral_key: session_init.as_ref().map(|s| s.ephemeral_public_key.clone()).unwrap_or_default(),
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

/// Insert a message into the `pending_messages` table for later retry.
async fn queue_pending_message(
    state: &Arc<AppState>,
    pool: &DbPool,
    recipient_key: &str,
    body: &str,
) -> Result<(), String> {
    let owner_key = state
        .identity
        .read()
        .as_ref()
        .map(|id| id.public_key.clone())
        .unwrap_or_default();
    let pool = pool.clone();
    let recipient = recipient_key.to_string();
    let body = body.to_string();
    let now = crate::db::timestamp_now();
    tokio::task::spawn_blocking(move || {
        let conn = pool.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO pending_messages (owner_key, recipient_key, body, created_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![owner_key, recipient, body, now],
        )
        .map_err(|e| format!("queue pending message: {e}"))?;
        Ok::<(), String>(())
    })
    .await
    .map_err(|e| e.to_string())?
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
        (pk, nh.routing_context.clone(), nh.profile_owner_keypair.clone())
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

    let value = serde_json::to_vec(&friend_keys)
        .map_err(|e| format!("serialize friend list: {e}"))?;

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
