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
    app_handle: tauri::AppHandle,
    mut shutdown_rx: mpsc::Receiver<()>,
) {
    tracing::info!("sync service started");

    let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
    let mut watched_keys: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut first_tick = true;
    let mut tick_count: u32 = 0;

    loop {
        tokio::select! {
            _ = interval.tick() => {
                tick_count += 1;
                if let Err(e) = sync_friends(&state, &app_handle, &mut watched_keys, first_tick).await {
                    tracing::warn!(error = %e, "friend sync failed");
                }
                first_tick = false;
                // After the first 3 rapid ticks, switch to the normal 30s cadence
                if tick_count == 3 {
                    interval = tokio::time::interval(std::time::Duration::from_secs(30));
                }
                if let Err(e) = sync_conversations(&state).await {
                    tracing::warn!(error = %e, "conversation sync failed");
                }
                if let Err(e) = sync_communities(&state, &pool).await {
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
pub async fn sync_friends_now(state: &Arc<AppState>, app_handle: &tauri::AppHandle) -> Result<(), String> {
    let mut watched_keys = std::collections::HashSet::new();
    sync_friends(state, app_handle, &mut watched_keys, true).await
}

/// Sync friend list: read friend profile DHT records and update local state.
///
/// Reads subkeys 2 (status), 4 (game info), 5 (prekey bundle), and 6 (route blob)
/// for each friend with a DHT record key. Also sets up DHT watches on first encounter.
async fn sync_friends(
    state: &Arc<AppState>,
    app_handle: &tauri::AppHandle,
    watched_keys: &mut std::collections::HashSet<String>,
    first_tick: bool,
) -> Result<(), String> {
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

    // Collect unwatched friend keys for force_refresh polling (per Veilid GitLab #377)
    let unwatched: std::collections::HashSet<String> = state.unwatched_friends.read().clone();

    // Clear watched_keys for friends whose watches died, so they get re-watched
    {
        let friends = state.friends.read();
        for fk in &unwatched {
            if let Some(f) = friends.get(fk.as_str()) {
                if let Some(ref dk) = f.dht_record_key {
                    watched_keys.remove(dk);
                }
            }
        }
    }

    for (friend_key, dht_key) in &friends_with_dht {
        let record_key: veilid_core::RecordKey = match dht_key.parse() {
            Ok(k) => k,
            Err(_) => continue,
        };

        // Register DHT key mapping (idempotent, no network call)
        {
            let mut dht_mgr = state.dht_manager.write();
            if let Some(mgr) = dht_mgr.as_mut() {
                mgr.register_friend_dht_key(dht_key.clone(), friend_key.clone());
            }
        }

        // Watch once per session; retry on next tick if failed
        if !watched_keys.contains(dht_key)
            && super::presence_service::watch_friend(state, friend_key, dht_key).await.is_ok()
        {
            watched_keys.insert(dht_key.clone());
        }

        // Force refresh for unwatched friends (watch failed — must poll from network)
        let force_refresh = first_tick || unwatched.contains(friend_key.as_str());
        sync_friend_dht_subkeys(state, &routing_context, friend_key, record_key, app_handle, force_refresh).await;
    }

    // Check for stale presences (friends whose heartbeat is expired)
    check_stale_presences(state, app_handle);

    tracing::debug!(friends = friends_with_dht.len(), "friend sync complete");
    Ok(())
}

/// Scan friends for stale heartbeats and mark them offline.
///
/// Called at the end of each `sync_friends()` tick. If a friend's
/// `last_heartbeat_at` is older than `STALE_PRESENCE_THRESHOLD_MS`,
/// they're treated as offline (crash without clean shutdown).
fn check_stale_presences(state: &Arc<AppState>, app_handle: &tauri::AppHandle) {
    use tauri::Emitter;
    let now = crate::db::timestamp_now();
    let threshold = super::presence_service::STALE_PRESENCE_THRESHOLD_MS;

    // Collect stale friends while holding the read lock
    let stale_friends: Vec<String> = {
        let friends = state.friends.read();
        friends
            .values()
            .filter(|f| {
                f.status != crate::state::UserStatus::Offline
                    && f.last_heartbeat_at
                        .is_some_and(|ts| now - ts > threshold)
            })
            .map(|f| f.public_key.clone())
            .collect()
    };

    // Mark stale friends offline and emit events (no lock held during emit)
    for pk in stale_friends {
        {
            let mut friends = state.friends.write();
            if let Some(friend) = friends.get_mut(&pk) {
                tracing::info!(friend = %pk, "stale heartbeat — marking offline");
                friend.status = crate::state::UserStatus::Offline;
                friend.last_seen_at = Some(now);
            }
        }
        let _ = app_handle.emit(
            "presence-event",
            &crate::channels::PresenceEvent::FriendOffline {
                public_key: pk,
            },
        );
    }
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
    app_handle: &tauri::AppHandle,
    force_refresh: bool,
) {
    // Ensure the record is open before reading (re-opening is a no-op if already open).
    if let Err(e) = routing_context.open_dht_record(record_key.clone(), None).await {
        tracing::debug!(
            friend = %friend_key, error = %e,
            "failed to open friend DHT record — will retry next tick"
        );
        return;
    }

    sync_friend_status(state, routing_context, friend_key, &record_key, app_handle, force_refresh).await;
    sync_friend_game_info(state, routing_context, friend_key, &record_key, app_handle, force_refresh).await;
    sync_friend_prekey(state, routing_context, friend_key, &record_key, force_refresh).await;
    sync_friend_route_blob(state, routing_context, friend_key, record_key, force_refresh).await;
}

/// Read status (subkey 2) from DHT and emit presence events on change.
///
/// Uses the 9-byte format `[status_byte, timestamp_be(8)]` with stale detection.
async fn sync_friend_status(
    state: &Arc<AppState>,
    routing_context: &veilid_core::RoutingContext,
    friend_key: &str,
    record_key: &veilid_core::RecordKey,
    app_handle: &tauri::AppHandle,
    force_refresh: bool,
) {
    use tauri::Emitter;
    use super::presence_service::{parse_status, parse_status_timestamp, STALE_PRESENCE_THRESHOLD_MS};

    let Some(value_data) = routing_context
        .get_dht_value(record_key.clone(), 2, force_refresh)
        .await
        .ok()
        .flatten()
    else {
        return;
    };

    let data = value_data.data();
    let Some(mut status) = parse_status(data) else {
        return;
    };

    // Store heartbeat timestamp
    let heartbeat_ts = parse_status_timestamp(data);
    if let Some(ts) = heartbeat_ts {
        let mut friends = state.friends.write();
        if let Some(friend) = friends.get_mut(friend_key) {
            friend.last_heartbeat_at = Some(ts);
        }
    }

    // Stale override: if timestamp > 5 min old, treat as offline
    if status != crate::state::UserStatus::Offline {
        if let Some(ts) = heartbeat_ts {
            let now = crate::db::timestamp_now();
            if now - ts > STALE_PRESENCE_THRESHOLD_MS {
                tracing::info!(friend = %friend_key, age_ms = now - ts, "stale heartbeat — treating as offline");
                status = crate::state::UserStatus::Offline;
            }
        }
    }

    // Compare with old status and emit events
    let old_status = {
        let friends = state.friends.read();
        friends.get(friend_key).map(|f| f.status)
    };
    {
        let mut friends = state.friends.write();
        if let Some(friend) = friends.get_mut(friend_key) {
            friend.status = status;
        }
    }
    if old_status != Some(status) {
        if status == crate::state::UserStatus::Offline {
            let _ = app_handle.emit(
                "presence-event",
                &crate::channels::PresenceEvent::FriendOffline {
                    public_key: friend_key.to_string(),
                },
            );
        } else {
            if old_status == Some(crate::state::UserStatus::Offline) {
                let _ = app_handle.emit(
                    "presence-event",
                    &crate::channels::PresenceEvent::FriendOnline {
                        public_key: friend_key.to_string(),
                    },
                );
            }
            let _ = app_handle.emit(
                "presence-event",
                &crate::channels::PresenceEvent::StatusChanged {
                    public_key: friend_key.to_string(),
                    status: format!("{status:?}").to_lowercase(),
                    status_message: None,
                },
            );
        }
    }
}

/// Read game info (subkey 4) from DHT and emit `GameChanged` on change.
async fn sync_friend_game_info(
    state: &Arc<AppState>,
    routing_context: &veilid_core::RoutingContext,
    friend_key: &str,
    record_key: &veilid_core::RecordKey,
    app_handle: &tauri::AppHandle,
    force_refresh: bool,
) {
    use tauri::Emitter;

    if let Ok(Some(value_data)) = routing_context
        .get_dht_value(record_key.clone(), 4, force_refresh)
        .await
    {
        let data = value_data.data();
        if !data.is_empty() {
            let game_info: Option<crate::state::GameInfoState> =
                serde_json::from_slice(data).ok();
            let old_game_name = {
                let friends = state.friends.read();
                friends.get(friend_key).and_then(|f| f.game_info.as_ref().map(|g| g.game_name.clone()))
            };
            let new_game_name = game_info.as_ref().map(|g| g.game_name.clone());
            {
                let mut friends = state.friends.write();
                if let Some(friend) = friends.get_mut(friend_key) {
                    friend.game_info.clone_from(&game_info);
                }
            }
            if old_game_name != new_game_name {
                let _ = app_handle.emit("presence-event",
                    &crate::channels::PresenceEvent::GameChanged {
                        public_key: friend_key.to_string(),
                        game_name: game_info.as_ref().map(|g| g.game_name.clone()),
                        game_id: game_info.as_ref().map(|g| g.game_id),
                        elapsed_seconds: None,
                    });
            }
        }
    }
}

/// Read prekey bundle (subkey 5) from DHT.
///
/// We no longer establish Signal sessions from DHT prekey bundles during sync.
/// Sessions are established during the friend request accept flow:
/// - Acceptor calls `establish_session()` (initiator) and sends ephemeral key
/// - Requester calls `respond_to_session()` (responder) with that ephemeral key
///
/// This function still reads the subkey (useful for future prekey rotation).
async fn sync_friend_prekey(
    _state: &Arc<AppState>,
    routing_context: &veilid_core::RoutingContext,
    friend_key: &str,
    record_key: &veilid_core::RecordKey,
    force_refresh: bool,
) {
    if let Ok(Some(value_data)) = routing_context
        .get_dht_value(record_key.clone(), 5, force_refresh)
        .await
    {
        let data = value_data.data();
        if !data.is_empty() {
            tracing::trace!(
                friend = %friend_key,
                prekey_len = data.len(),
                "read prekey bundle from DHT (session established via friend accept flow)"
            );
        }
    }
}

/// Read route blob (subkey 6) from DHT and cache it.
async fn sync_friend_route_blob(
    state: &Arc<AppState>,
    routing_context: &veilid_core::RoutingContext,
    friend_key: &str,
    record_key: veilid_core::RecordKey,
    force_refresh: bool,
) {
    if let Ok(Some(value_data)) = routing_context
        .get_dht_value(record_key, 6, force_refresh)
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

    let Some(secret_bytes) = *state.identity_secret.lock() else {
        return Ok(()); // Not logged in
    };

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
        sync_single_conversation(state, &routing_context, &secret_bytes, friend_key, remote_conv_key).await;
    }

    if !friends_with_conversations.is_empty() {
        tracing::debug!(
            conversations = friends_with_conversations.len(),
            "conversation sync complete"
        );
    }

    Ok(())
}

/// Sync a single friend's conversation record from DHT.
async fn sync_single_conversation(
    state: &Arc<AppState>,
    routing_context: &veilid_core::RoutingContext,
    my_secret_bytes: &[u8; 32],
    friend_key: &str,
    remote_conv_key: &str,
) {
    let my_identity = rekindle_crypto::Identity::from_secret_bytes(my_secret_bytes);
    let my_x25519_secret = my_identity.to_x25519_secret();

    // Derive the DH conversation encryption key
    let Ok(friend_ed_bytes) = hex::decode(friend_key) else {
        return;
    };
    let Ok(friend_ed_array): Result<[u8; 32], _> = friend_ed_bytes.try_into() else {
        return;
    };
    let friend_identity = rekindle_crypto::Identity::from_secret_bytes(&friend_ed_array);
    let friend_x25519_public = friend_identity.to_x25519_public();

    let encryption_key =
        rekindle_crypto::DhtRecordKey::derive_conversation_key(&my_x25519_secret, &friend_x25519_public);

    // Open the remote conversation record read-only
    let record = match rekindle_protocol::dht::conversation::ConversationRecord::open_read(
        routing_context,
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
            return;
        }
    };

    // Track the opened record key for cleanup on logout/exit
    {
        let mut dht_mgr = state.dht_manager.write();
        if let Some(ref mut mgr) = dht_mgr.as_mut() {
            mgr.track_open_record(remote_conv_key.to_string());
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

/// Parse a DHT channel list from JSON (supports both wrapped and bare array format).
fn parse_dht_channel_list(data: &[u8]) -> Vec<crate::state::ChannelInfo> {
    let channel_list: Vec<serde_json::Value> = match serde_json::from_slice::<serde_json::Value>(data) {
        Ok(v) => {
            if let Some(obj) = v.as_object() {
                obj.get("channels").and_then(|c| c.as_array().cloned()).unwrap_or_default()
            } else {
                v.as_array().cloned().unwrap_or_default()
            }
        }
        Err(_) => return vec![],
    };
    channel_list
        .iter()
        .filter_map(|ch| {
            let id = ch.get("id")?.as_str()?.to_string();
            let name = ch.get("name")?.as_str()?.to_string();
            let ch_type = match ch.get("channelType").and_then(|v| v.as_str()) {
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
        .collect()
}

/// Sync communities: read community DHT records and update local state.
///
/// Reads metadata (subkey 0), channel list (subkey 1), and server route
/// (subkey 6) from each community's DHT record.
async fn sync_communities(state: &Arc<AppState>, pool: &DbPool) -> Result<(), String> {
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
                let channels = parse_dht_channel_list(&data);
                if !channels.is_empty() {
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

        // Read server route subkey (6) from DHT — ensures route survives restarts
        sync_community_server_route(state, pool, &mgr, community_id, dht_key).await;
    }

    tracing::debug!(communities = communities_with_dht.len(), "community sync complete");
    Ok(())
}

/// Read server route (subkey 6) from DHT and persist to `SQLite` if changed.
async fn sync_community_server_route(
    state: &Arc<AppState>,
    pool: &DbPool,
    mgr: &rekindle_protocol::dht::DHTManager,
    community_id: &str,
    dht_key: &str,
) {
    let route_blob = match mgr
        .get_value(dht_key, rekindle_protocol::dht::community::SUBKEY_SERVER_ROUTE)
        .await
    {
        Ok(Some(blob)) => blob,
        Ok(None) => return,
        Err(e) => {
            tracing::trace!(
                community = %community_id,
                error = %e,
                "failed to read server route from DHT"
            );
            return;
        }
    };

    let needs_update = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .is_some_and(|c| c.server_route_blob.as_deref() != Some(&route_blob))
    };
    if !needs_update {
        return;
    }

    {
        let mut communities = state.communities.write();
        if let Some(community) = communities.get_mut(community_id) {
            community.server_route_blob = Some(route_blob.clone());
        }
    }

    // Persist to SQLite
    let owner_key = state
        .identity
        .read()
        .as_ref()
        .map(|id| id.public_key.clone())
        .unwrap_or_default();
    let pool_clone = pool.clone();
    let cid = community_id.to_string();
    let _ = tokio::task::spawn_blocking(move || {
        let conn = pool_clone.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE communities SET server_route_blob = ?1 WHERE owner_key = ?2 AND id = ?3",
            rusqlite::params![route_blob, owner_key, cid],
        )
        .map_err(|e| format!("sync persist server_route_blob: {e}"))?;
        Ok::<(), String>(())
    })
    .await;
    tracing::debug!(community = %community_id, "synced server route from DHT to SQLite");
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
        retry_single_pending(state, pool, id, &recipient_key, &body, retry_count, MAX_RETRIES).await?;
    }

    Ok(())
}

/// Attempt to deliver a single pending message, dropping or incrementing on failure.
async fn retry_single_pending(
    state: &Arc<AppState>,
    pool: &DbPool,
    id: i64,
    recipient_key: &str,
    body: &str,
    retry_count: i64,
    max_retries: i64,
) -> Result<(), String> {
    if retry_count >= max_retries {
        tracing::warn!(
            id,
            to = %recipient_key,
            retries = retry_count,
            "pending message exceeded max retries — dropping"
        );
        delete_pending_message(pool, id).await?;
        return Ok(());
    }

    // Try DM envelope first (existing logic), then channel message retry
    if let Ok(envelope) = serde_json::from_str::<MessageEnvelope>(body) {
        retry_pending_dm(state, pool, id, recipient_key, &envelope).await?;
    } else if let Ok(channel_msg) =
        serde_json::from_str::<crate::commands::community::PendingChannelMessage>(body)
    {
        // Channel message retry — look up server route from community state
        let route_blob = {
            let communities = state.communities.read();
            communities
                .get(&channel_msg.community_id)
                .and_then(|c| c.server_route_blob.clone())
        };
        let Some(route_blob) = route_blob else {
            increment_retry_count(pool, id).await?;
            return Ok(());
        };

        match crate::commands::community::send_encrypted_to_server(
            state,
            &channel_msg.channel_id,
            &channel_msg.community_id,
            channel_msg.ciphertext,
            channel_msg.mek_generation,
            channel_msg.timestamp,
            route_blob,
        )
        .await
        {
            Ok(()) => {
                tracing::debug!(id, "pending channel message delivered");
                delete_pending_message(pool, id).await?;
            }
            Err(e) => {
                tracing::debug!(id, error = %e, "pending channel message retry failed");
                increment_retry_count(pool, id).await?;
            }
        }
    } else {
        tracing::warn!(id, "unrecognized pending message format — dropping");
        delete_pending_message(pool, id).await?;
    }

    Ok(())
}

/// Retry delivering a single pending DM envelope via cached route or mailbox fallback.
async fn retry_pending_dm(
    state: &Arc<AppState>,
    pool: &DbPool,
    id: i64,
    recipient_key: &str,
    envelope: &MessageEnvelope,
) -> Result<(), String> {
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
            return Ok(());
        };

        let mut dht_mgr = state.dht_manager.write();
        match dht_mgr.as_mut() {
            Some(mgr) => {
                match mgr.manager.get_cached_route(recipient_key).cloned() {
                    Some(blob) => match mgr.manager.get_or_import_route(&api, &blob) {
                        Ok(route_id) => Some((route_id, rc)),
                        Err(e) => {
                            tracing::debug!(
                                to = %recipient_key, error = %e, blob_len = blob.len(),
                                "route import failed during retry — invalidating, will try mailbox"
                            );
                            mgr.manager.invalidate_route_for_peer(recipient_key);
                            None
                        }
                    },
                    None => None,
                }
            }
            None => None,
        }
    };

    // If no cached route, try mailbox fallback
    let route_id_and_rc = if route_id_and_rc.is_some() {
        route_id_and_rc
    } else {
        try_mailbox_route_fallback(state, recipient_key).await
    };

    let Some((route_id, routing_context)) = route_id_and_rc else {
        increment_retry_count(pool, id).await?;
        return Ok(());
    };

    match rekindle_protocol::messaging::sender::send_envelope(
        &routing_context,
        route_id,
        envelope,
    )
    .await
    {
        Ok(()) => {
            tracing::debug!(id, to = %recipient_key, "pending DM delivered successfully");
            delete_pending_message(pool, id).await?;
        }
        Err(e) => {
            tracing::debug!(id, to = %recipient_key, error = %e, "pending DM retry failed");
            increment_retry_count(pool, id).await?;
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

/// Try to discover a peer's route via their mailbox DHT record.
///
/// If the friend has a `mailbox_dht_key`, read subkey 0 (route blob), cache it,
/// and import it. Returns `Some((RouteId, RoutingContext))` on success.
async fn try_mailbox_route_fallback(
    state: &Arc<AppState>,
    recipient_key: &str,
) -> Option<(veilid_core::RouteId, veilid_core::RoutingContext)> {
    // Look up the friend's mailbox key
    let mailbox_key = {
        let friends = state.friends.read();
        friends
            .get(recipient_key)
            .and_then(|f| f.mailbox_dht_key.clone())
    }?;

    let rc = {
        let node = state.node.read();
        node.as_ref().map(|nh| nh.routing_context.clone())
    }?;

    // Read fresh route blob from mailbox
    let route_blob =
        match rekindle_protocol::dht::mailbox::read_peer_mailbox_route(&rc, &mailbox_key).await {
            Ok(Some(blob)) if !blob.is_empty() => blob,
            Ok(_) => {
                tracing::trace!(to = %recipient_key, "mailbox route blob empty or missing");
                return None;
            }
            Err(e) => {
                tracing::trace!(to = %recipient_key, error = %e, "failed to read mailbox");
                return None;
            }
        };

    // Cache and import
    let api = {
        let node = state.node.read();
        node.as_ref().map(|nh| nh.api.clone())
    }?;

    let mut dht_mgr = state.dht_manager.write();
    let mgr = dht_mgr.as_mut()?;
    mgr.manager
        .cache_route(&api, recipient_key, route_blob.clone());
    match mgr.manager.get_or_import_route(&api, &route_blob) {
        Ok(route_id) => {
            tracing::debug!(to = %recipient_key, "discovered route via mailbox fallback");
            Some((route_id, rc))
        }
        Err(e) => {
            tracing::trace!(to = %recipient_key, error = %e, "failed to import mailbox route");
            None
        }
    }
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
