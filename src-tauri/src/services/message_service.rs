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

    // Step 2: Decrypt payload if Signal session exists, otherwise use raw payload
    let payload_bytes = {
        let signal = state.signal_manager.lock();
        if let Some(handle) = signal.as_ref() {
            match handle.manager.has_session(&sender_hex) {
                Ok(true) => match handle.manager.decrypt(&sender_hex, &envelope.payload) {
                    Ok(pt) => pt,
                    Err(e) => {
                        tracing::warn!(
                            error = %e, from = %sender_hex,
                            "Signal decrypt failed — trying plaintext fallback"
                        );
                        envelope.payload.clone()
                    }
                },
                _ => envelope.payload.clone(),
            }
        } else {
            envelope.payload.clone()
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

    // Non-friend filtering: only FriendRequest allowed from non-friends
    if !matches!(payload, MessagePayload::FriendRequest { .. })
        && !state.friends.read().contains_key(&sender_hex)
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
        MessagePayload::FriendAccept { prekey_bundle, profile_dht_key, route_blob, mailbox_dht_key } => {
            handle_friend_accept_full(
                app_handle, state, pool, &sender_hex, &prekey_bundle,
                &profile_dht_key, route_blob, &mailbox_dht_key,
            ).await;
        }
        MessagePayload::FriendReject => {
            let _ = app_handle.emit("chat-event", &ChatEvent::FriendRequestRejected { from: sender_hex });
        }
        MessagePayload::ProfileKeyRotated { new_profile_dht_key } => {
            handle_profile_key_rotated(state, pool, &sender_hex, &new_profile_dht_key).await;
        }
        MessagePayload::PresenceUpdate { .. } => {}
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

/// Process incoming friend request: establish Signal session from their `PreKeyBundle`.
fn handle_friend_request(
    state: &Arc<AppState>,
    sender_hex: &str,
    _display_name: &str,
    _message: &str,
    prekey_bundle_bytes: &[u8],
) {
    // Deserialize the sender's PreKeyBundle and establish a Signal session
    // so we can send encrypted messages to them in the future
    match serde_json::from_slice::<rekindle_crypto::signal::PreKeyBundle>(prekey_bundle_bytes) {
        Ok(bundle) => {
            let signal = state.signal_manager.lock();
            if let Some(handle) = signal.as_ref() {
                match handle.manager.establish_session(sender_hex, &bundle) {
                    Ok(()) => {
                        tracing::info!(from = %sender_hex, "established Signal session from friend request");
                    }
                    Err(e) => {
                        tracing::warn!(
                            from = %sender_hex, error = %e,
                            "failed to establish Signal session from friend request"
                        );
                    }
                }
            }
        }
        Err(e) => {
            tracing::warn!(
                from = %sender_hex, error = %e,
                "failed to deserialize PreKeyBundle from friend request"
            );
        }
    }
}

/// Process friend accept: establish Signal session from their `PreKeyBundle`.
fn handle_friend_accept(
    state: &Arc<AppState>,
    sender_hex: &str,
    prekey_bundle_bytes: &[u8],
) {
    match serde_json::from_slice::<rekindle_crypto::signal::PreKeyBundle>(prekey_bundle_bytes) {
        Ok(bundle) => {
            let signal = state.signal_manager.lock();
            if let Some(handle) = signal.as_ref() {
                match handle.manager.establish_session(sender_hex, &bundle) {
                    Ok(()) => {
                        tracing::info!(from = %sender_hex, "established Signal session from friend accept");
                    }
                    Err(e) => {
                        tracing::warn!(
                            from = %sender_hex, error = %e,
                            "failed to establish Signal session from friend accept"
                        );
                    }
                }
            }
        }
        Err(e) => {
            tracing::warn!(
                from = %sender_hex, error = %e,
                "failed to deserialize PreKeyBundle from friend accept"
            );
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
    handle_friend_request(state, sender_hex, display_name, message, prekey_bundle);

    // If sender is already our friend (bidirectional add), update their display name
    let is_existing_friend = state.friends.read().contains_key(sender_hex);
    if is_existing_friend {
        {
            let mut friends = state.friends.write();
            if let Some(friend) = friends.get_mut(sender_hex) {
                friend.display_name = display_name.to_string();
            }
        }
        update_friend_display_name(state, pool, sender_hex, display_name).await;
    }

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
    persist_friend_request(
        state, pool, sender_hex, display_name, message,
        profile_dht_key, route_blob, mailbox_dht_key,
    )
    .await;
    let event = ChatEvent::FriendRequest {
        from: sender_hex.to_string(),
        display_name: display_name.to_string(),
        message: message.to_string(),
    };
    let _ = app_handle.emit("chat-event", &event);
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
) {
    handle_friend_accept(state, sender_hex, prekey_bundle);
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
    // Store profile key and mailbox key on friend state
    {
        let mut friends = state.friends.write();
        if let Some(friend) = friends.get_mut(sender_hex) {
            if !profile_dht_key.is_empty() {
                friend.dht_record_key = Some(profile_dht_key.to_string());
            }
            if !mailbox_dht_key.is_empty() {
                friend.mailbox_dht_key = Some(mailbox_dht_key.to_string());
            }
        }
    }
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
    let now = crate::db::timestamp_now();
    if let Err(e) = tokio::task::spawn_blocking(move || {
        let conn = pool.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT OR IGNORE INTO pending_friend_requests \
             (owner_key, public_key, display_name, message, received_at, profile_dht_key, route_blob, mailbox_dht_key) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![owner_key, pk, dn, msg, now, pdk, rb, mdk],
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

/// Build a `MessageEnvelope`, optionally encrypt with Signal, and send via Veilid.
///
/// If no route exists for the peer, the message is queued for retry by `sync_service`.
async fn send_envelope_to_peer(
    state: &Arc<AppState>,
    pool: &DbPool,
    to: &str,
    payload: &MessagePayload,
    encrypt: bool,
) -> Result<(), String> {
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
                let route_id = mgr
                    .manager
                    .get_or_import_route(&api, &blob)
                    .map_err(|e| format!("import route: {e}"))?;
                Some((route_id, rc))
            }
            None => None,
        }
    };

    let Some((route_id, routing_context)) = route_id_and_rc else {
        // No cached route — serialize the envelope and queue for later delivery
        tracing::debug!(to = %to, "no cached route for peer — queuing message for retry");
        let envelope_json =
            serde_json::to_string(&envelope).map_err(|e| format!("serialize envelope: {e}"))?;
        queue_pending_message(state, pool, to, &envelope_json).await?;
        return Ok(());
    };

    send_envelope(&routing_context, route_id, &envelope)
        .await
        .map_err(|e| format!("send_envelope: {e}"))?;

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
/// Includes our `PreKeyBundle` so the requester can establish a Signal session back.
pub async fn send_friend_accept(
    state: &Arc<AppState>,
    pool: &DbPool,
    to: &str,
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

    let payload = MessagePayload::FriendAccept {
        prekey_bundle,
        profile_dht_key,
        route_blob,
        mailbox_dht_key,
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
        let keys: Vec<String> = friends.keys().cloned().collect();
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
