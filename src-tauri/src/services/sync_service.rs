use std::sync::Arc;

use rekindle_protocol::messaging::envelope::MessageEnvelope;
use tokio::sync::mpsc;

use crate::db::DbPool;
use crate::state::AppState;

/// Start the periodic sync service.
///
/// Runs in the background, periodically syncing local `SQLite` cache with Veilid DHT.
/// - Pull: Read latest from DHT -> update `SQLite`
/// - Push: Send local changes to DHT
/// - Retry: Attempt to deliver queued pending messages
pub async fn start_sync_loop(
    state: Arc<AppState>,
    pool: DbPool,
    mut shutdown_rx: mpsc::Receiver<()>,
) {
    tracing::info!("sync service started");

    let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));

    loop {
        tokio::select! {
            _ = interval.tick() => {
                if let Err(e) = sync_friends(&state).await {
                    tracing::warn!(error = %e, "friend sync failed");
                }
                if let Err(e) = sync_conversations(&state).await {
                    tracing::warn!(error = %e, "conversation sync failed");
                }
                if let Err(e) = sync_communities(&state).await {
                    tracing::warn!(error = %e, "community sync failed");
                }
                if let Err(e) = retry_pending_messages(&state, &pool).await {
                    tracing::warn!(error = %e, "pending message retry failed");
                }
            }
            _ = shutdown_rx.recv() => {
                tracing::info!("sync service shutting down");
                break;
            }
        }
    }
}

/// Run a single friend sync immediately (called from auth on login).
///
/// This avoids waiting 30 seconds for the first periodic tick.
pub async fn sync_friends_now(state: &Arc<AppState>) -> Result<(), String> {
    sync_friends(state).await
}

/// Sync friend list: read friend profile DHT records and update local state.
///
/// Reads subkeys 2 (status), 4 (game info), 5 (prekey bundle), and 6 (route blob)
/// for each friend with a DHT record key. Also sets up DHT watches on first encounter.
async fn sync_friends(state: &Arc<AppState>) -> Result<(), String> {
    // Clone routing_context out before any await (parking_lot guards are !Send)
    let routing_context = {
        let node = state.node.read();
        match node.as_ref() {
            Some(nh) if nh.is_attached => Some(nh.routing_context.clone()),
            _ => None,
        }
    };

    let Some(routing_context) = routing_context else {
        return Ok(()); // Not connected yet
    };

    // Collect friends that have DHT record keys
    let friends_with_dht: Vec<(String, String)> = {
        let friends = state.friends.read();
        friends
            .values()
            .filter_map(|f| {
                f.dht_record_key
                    .as_ref()
                    .map(|k| (f.public_key.clone(), k.clone()))
            })
            .collect()
    };

    for (friend_key, dht_key) in &friends_with_dht {
        let record_key: veilid_core::RecordKey = match dht_key.parse() {
            Ok(k) => k,
            Err(_) => continue,
        };

        // Ensure the DHT key → friend mapping is registered for value change routing.
        // On the first encounter, also start a DHT watch for real-time presence.
        let needs_watch = {
            let mut dht_mgr = state.dht_manager.write();
            if let Some(mgr) = dht_mgr.as_mut() {
                if mgr.friend_for_dht_key(dht_key).is_none() {
                    mgr.register_friend_dht_key(dht_key.clone(), friend_key.clone());
                    true
                } else {
                    false
                }
            } else {
                false
            }
        };
        if needs_watch {
            if let Err(e) =
                super::presence_service::watch_friend(state, friend_key, dht_key).await
            {
                tracing::trace!(friend = %friend_key, error = %e, "failed to watch friend DHT");
            }
        }

        sync_friend_dht_subkeys(state, &routing_context, friend_key, record_key).await;
    }

    tracing::debug!(friends = friends_with_dht.len(), "friend sync complete");
    Ok(())
}

/// Read DHT subkeys for a single friend and update local state.
///
/// Reads subkeys 2 (status), 4 (game info), 5 (prekey bundle), and 6 (route blob)
/// from the friend's DHT profile record.
async fn sync_friend_dht_subkeys(
    state: &Arc<AppState>,
    routing_context: &veilid_core::RoutingContext,
    friend_key: &str,
    record_key: veilid_core::RecordKey,
) {
    // Ensure the record is open before reading (re-opening is a no-op if already open).
    if let Err(e) = routing_context.open_dht_record(record_key.clone(), None).await {
        tracing::trace!(
            friend = %friend_key, error = %e,
            "failed to open friend DHT record for sync — skipping"
        );
        return;
    }

    // Read status subkey (2) from DHT
    if let Ok(Some(value_data)) = routing_context
        .get_dht_value(record_key.clone(), 2, false)
        .await
    {
        if let Some(&status_byte) = value_data.data().first() {
            let status = match status_byte {
                0 => crate::state::UserStatus::Online,
                1 => crate::state::UserStatus::Away,
                2 => crate::state::UserStatus::Busy,
                _ => crate::state::UserStatus::Offline,
            };
            let mut friends = state.friends.write();
            if let Some(friend) = friends.get_mut(friend_key) {
                friend.status = status;
            }
        }
    }

    // Read game info subkey (4) from DHT — fallback for when DHT watches fail
    if let Ok(Some(value_data)) = routing_context
        .get_dht_value(record_key.clone(), 4, false)
        .await
    {
        let data = value_data.data();
        if !data.is_empty() {
            let game_info: Option<crate::state::GameInfoState> =
                serde_json::from_slice(data).ok();
            let mut friends = state.friends.write();
            if let Some(friend) = friends.get_mut(friend_key) {
                friend.game_info = game_info;
            }
        }
    }

    // Read prekey bundle subkey (5) from DHT — cache for Signal session establishment
    if let Ok(Some(value_data)) = routing_context
        .get_dht_value(record_key.clone(), 5, false)
        .await
    {
        let data = value_data.data();
        if !data.is_empty() {
            // Try to establish a Signal session if we don't have one yet
            let has_session = {
                let signal = state.signal_manager.lock();
                signal
                    .as_ref()
                    .and_then(|h| h.manager.has_session(friend_key).ok())
                    .unwrap_or(false)
            };
            if !has_session {
                if let Ok(bundle) =
                    serde_json::from_slice::<rekindle_crypto::signal::PreKeyBundle>(data)
                {
                    let signal = state.signal_manager.lock();
                    if let Some(handle) = signal.as_ref() {
                        match handle.manager.establish_session(friend_key, &bundle) {
                            Ok(()) => {
                                tracing::info!(
                                    friend = %friend_key,
                                    "established Signal session from DHT prekey bundle"
                                );
                            }
                            Err(e) => {
                                tracing::trace!(
                                    friend = %friend_key, error = %e,
                                    "failed to establish Signal session from DHT"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    // Read route blob subkey (6) from DHT and cache it
    if let Ok(Some(value_data)) = routing_context
        .get_dht_value(record_key, 6, false)
        .await
    {
        let route_blob = value_data.data().to_vec();
        if !route_blob.is_empty() {
            let api = {
                let node = state.node.read();
                node.as_ref().map(|nh| nh.api.clone())
            };
            if let Some(api) = api {
                let mut dht_mgr = state.dht_manager.write();
                if let Some(mgr) = dht_mgr.as_mut() {
                    mgr.manager.cache_route(&api, friend_key, route_blob);
                }
            }
        }
    }
}

/// Sync conversation records for friends that have remote conversation keys.
///
/// For each friend with a `remote_conversation_key`, derives the DH shared secret,
/// opens the conversation record read-only, reads the header, and caches the
/// route blob and profile snapshot.
async fn sync_conversations(state: &Arc<AppState>) -> Result<(), String> {
    let routing_context = {
        let node = state.node.read();
        match node.as_ref() {
            Some(nh) if nh.is_attached => Some(nh.routing_context.clone()),
            _ => None,
        }
    };

    let Some(routing_context) = routing_context else {
        return Ok(()); // Not connected yet
    };

    let secret_bytes = match *state.identity_secret.lock() {
        Some(s) => s,
        None => return Ok(()), // Not logged in
    };

    let identity = rekindle_crypto::Identity::from_secret_bytes(&secret_bytes);
    let my_x25519_secret = identity.to_x25519_secret();

    // Collect friends that have remote conversation keys
    let friends_with_conversations: Vec<(String, String)> = {
        let friends = state.friends.read();
        friends
            .values()
            .filter_map(|f| {
                f.remote_conversation_key
                    .as_ref()
                    .map(|k| (f.public_key.clone(), k.clone()))
            })
            .collect()
    };

    for (friend_key, remote_conv_key) in &friends_with_conversations {
        // Derive the DH conversation encryption key
        let friend_ed_bytes = match hex::decode(friend_key) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let friend_ed_array: [u8; 32] = match friend_ed_bytes.try_into() {
            Ok(a) => a,
            Err(_) => continue,
        };
        let friend_identity = rekindle_crypto::Identity::from_secret_bytes(&friend_ed_array);
        let friend_x25519_public = friend_identity.to_x25519_public();

        let encryption_key =
            rekindle_crypto::DhtRecordKey::derive_conversation_key(&my_x25519_secret, &friend_x25519_public);

        // Open the remote conversation record read-only
        let record = match rekindle_protocol::dht::conversation::ConversationRecord::open_read(
            &routing_context,
            remote_conv_key,
            encryption_key,
        )
        .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::trace!(
                    friend = %friend_key, key = %remote_conv_key,
                    error = %e, "failed to open remote conversation record"
                );
                continue;
            }
        };

        // Track the opened record key for cleanup on logout/exit
        {
            let mut dht_mgr = state.dht_manager.write();
            if let Some(ref mut mgr) = dht_mgr.as_mut() {
                mgr.track_open_record(remote_conv_key.clone());
            }
        }

        // Read header and cache route blob + profile
        match record.read_header().await {
            Ok(header) => {
                // Cache route blob
                if !header.route_blob.is_empty() {
                    let api = {
                        let node = state.node.read();
                        node.as_ref().map(|nh| nh.api.clone())
                    };
                    if let Some(api) = api {
                        let mut dht_mgr = state.dht_manager.write();
                        if let Some(mgr) = dht_mgr.as_mut() {
                            mgr.manager.cache_route(&api, friend_key, header.route_blob);
                        }
                    }
                }

                // Update friend display name from conversation profile snapshot
                {
                    let mut friends = state.friends.write();
                    if let Some(friend) = friends.get_mut(friend_key) {
                        if !header.profile.display_name.is_empty() {
                            friend.display_name = header.profile.display_name;
                        }
                        if !header.profile.status_message.is_empty() {
                            friend.status_message = Some(header.profile.status_message);
                        }
                    }
                }
            }
            Err(e) => {
                tracing::trace!(
                    friend = %friend_key, key = %remote_conv_key,
                    error = %e, "failed to read remote conversation header"
                );
            }
        }

        // Best-effort close
        let _ = record.close().await;
    }

    if !friends_with_conversations.is_empty() {
        tracing::debug!(
            conversations = friends_with_conversations.len(),
            "conversation sync complete"
        );
    }

    Ok(())
}

/// Sync communities: read community DHT records and update local state.
async fn sync_communities(state: &Arc<AppState>) -> Result<(), String> {
    // Clone routing_context out before any await (parking_lot guards are !Send)
    let routing_context = {
        let node = state.node.read();
        match node.as_ref() {
            Some(nh) if nh.is_attached => Some(nh.routing_context.clone()),
            _ => None,
        }
    };

    let Some(routing_context) = routing_context else {
        return Ok(()); // Not connected yet
    };

    // Collect communities that have DHT record keys
    let communities_with_dht: Vec<(String, String)> = {
        let communities = state.communities.read();
        communities
            .values()
            .filter_map(|c| {
                c.dht_record_key
                    .as_ref()
                    .map(|k| (c.id.clone(), k.clone()))
            })
            .collect()
    };

    let mgr = rekindle_protocol::dht::DHTManager::new(routing_context);

    for (community_id, dht_key) in &communities_with_dht {
        // Ensure the record is open before reading (re-opening is a no-op if already open).
        if let Err(e) = mgr.open_record(dht_key).await {
            tracing::trace!(
                community = %community_id, error = %e,
                "failed to open community DHT record for sync — skipping"
            );
            continue;
        }

        // Read metadata subkey (0) from DHT
        match mgr.get_value(dht_key, 0).await {
            Ok(Some(data)) => {
                if let Ok(metadata) = serde_json::from_slice::<serde_json::Value>(&data) {
                    let mut communities = state.communities.write();
                    if let Some(community) = communities.get_mut(community_id) {
                        if let Some(name) = metadata.get("name").and_then(|v| v.as_str()) {
                            community.name = name.to_string();
                        }
                        if let Some(desc) = metadata.get("description").and_then(|v| v.as_str()) {
                            community.description = Some(desc.to_string());
                        }
                    }
                }
            }
            Ok(None) => {}
            Err(e) => {
                tracing::trace!(
                    community = %community_id,
                    error = %e,
                    "failed to read community metadata from DHT"
                );
            }
        }

        // Read channel list subkey (1) from DHT
        match mgr.get_value(dht_key, 1).await {
            Ok(Some(data)) => {
                if let Ok(channel_list) = serde_json::from_slice::<Vec<serde_json::Value>>(&data) {
                    let channels: Vec<crate::state::ChannelInfo> = channel_list
                        .iter()
                        .filter_map(|ch| {
                            let id = ch.get("id")?.as_str()?.to_string();
                            let name = ch.get("name")?.as_str()?.to_string();
                            let ch_type =
                                match ch.get("channelType").and_then(|v| v.as_str()) {
                                    Some("voice") => crate::state::ChannelType::Voice,
                                    _ => crate::state::ChannelType::Text,
                                };
                            Some(crate::state::ChannelInfo {
                                id,
                                name,
                                channel_type: ch_type,
                                unread_count: 0,
                            })
                        })
                        .collect();

                    let mut communities = state.communities.write();
                    if let Some(community) = communities.get_mut(community_id) {
                        community.channels = channels;
                    }
                }
            }
            Ok(None) => {}
            Err(e) => {
                tracing::trace!(
                    community = %community_id,
                    error = %e,
                    "failed to read community channels from DHT"
                );
            }
        }
    }

    tracing::debug!(communities = communities_with_dht.len(), "community sync complete");
    Ok(())
}

/// Retry sending queued pending messages.
///
/// Reads all rows from `pending_messages`, attempts to deliver each envelope
/// directly via Veilid (the body is a JSON-serialized `MessageEnvelope`).
/// Deletes on success, increments `retry_count` on failure.
/// Messages exceeding 20 retries (~10 minutes at 30 s intervals) are dropped.
async fn retry_pending_messages(state: &Arc<AppState>, pool: &DbPool) -> Result<(), String> {
    const MAX_RETRIES: i64 = 20;

    // Step 1: Read all pending messages from DB (scoped to current identity)
    let owner_key = state
        .identity
        .read()
        .as_ref()
        .map(|id| id.public_key.clone())
        .unwrap_or_default();
    let pending: Vec<(i64, String, String, i64)> = {
        let pool = pool.clone();
        let ok = owner_key;
        tokio::task::spawn_blocking(move || {
            let conn = pool.lock().map_err(|e| e.to_string())?;
            let mut stmt = conn
                .prepare("SELECT id, recipient_key, body, retry_count FROM pending_messages WHERE owner_key = ?1 ORDER BY id")
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map(rusqlite::params![ok], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                })
                .map_err(|e| e.to_string())?;
            let mut results = Vec::new();
            for row in rows {
                results.push(row.map_err(|e| e.to_string())?);
            }
            Ok::<_, String>(results)
        })
        .await
        .map_err(|e| e.to_string())??
    };

    if pending.is_empty() {
        return Ok(());
    }

    tracing::debug!(count = pending.len(), "retrying pending messages");

    for (id, recipient_key, body, retry_count) in pending {
        if retry_count >= MAX_RETRIES {
            tracing::warn!(
                id,
                to = %recipient_key,
                retries = retry_count,
                "pending message exceeded max retries — dropping"
            );
            delete_pending_message(pool, id).await?;
            continue;
        }

        // The body is a JSON-serialized MessageEnvelope — send it directly
        let envelope: MessageEnvelope = match serde_json::from_str(&body) {
            Ok(env) => env,
            Err(e) => {
                tracing::warn!(id, error = %e, "failed to parse pending message as envelope — dropping");
                delete_pending_message(pool, id).await?;
                continue;
            }
        };

        // Look up route and import RouteId via cache.
        // Clone Arc-based handles out before any .await (parking_lot guards are !Send).
        let route_id_and_rc = {
            let api_and_rc = {
                let node = state.node.read();
                node.as_ref()
                    .map(|nh| (nh.api.clone(), nh.routing_context.clone()))
            };
            let Some((api, rc)) = api_and_rc else {
                increment_retry_count(pool, id).await?;
                continue;
            };

            let mut dht_mgr = state.dht_manager.write();
            match dht_mgr.as_mut() {
                Some(mgr) => {
                    match mgr.manager.get_cached_route(&recipient_key).cloned() {
                        Some(blob) => match mgr.manager.get_or_import_route(&api, &blob) {
                            Ok(route_id) => Some((route_id, rc)),
                            Err(_) => None,
                        },
                        None => None,
                    }
                }
                None => None,
            }
        };

        let Some((route_id, routing_context)) = route_id_and_rc else {
            increment_retry_count(pool, id).await?;
            continue;
        };

        match rekindle_protocol::messaging::sender::send_envelope(
            &routing_context,
            route_id,
            &envelope,
        )
        .await
        {
            Ok(()) => {
                tracing::debug!(id, to = %recipient_key, "pending message delivered successfully");
                delete_pending_message(pool, id).await?;
            }
            Err(e) => {
                tracing::debug!(id, to = %recipient_key, error = %e, "pending message retry failed");
                increment_retry_count(pool, id).await?;
            }
        }
    }

    Ok(())
}

/// Delete a single pending message by ID.
async fn delete_pending_message(pool: &DbPool, id: i64) -> Result<(), String> {
    let pool = pool.clone();
    tokio::task::spawn_blocking(move || {
        let conn = pool.lock().map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM pending_messages WHERE id = ?1", rusqlite::params![id])
            .map_err(|e| format!("delete pending message: {e}"))?;
        Ok::<(), String>(())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Increment the `retry_count` for a pending message.
async fn increment_retry_count(pool: &DbPool, id: i64) -> Result<(), String> {
    let pool = pool.clone();
    tokio::task::spawn_blocking(move || {
        let conn = pool.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE pending_messages SET retry_count = retry_count + 1 WHERE id = ?1",
            rusqlite::params![id],
        )
        .map_err(|e| format!("increment retry count: {e}"))?;
        Ok::<(), String>(())
    })
    .await
    .map_err(|e| e.to_string())?
}
