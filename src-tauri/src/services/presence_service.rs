use std::sync::Arc;

use tauri::{Emitter, Manager};

use crate::channels::PresenceEvent;
use crate::db::{self, DbPool};
use crate::state::{AppState, GameInfoState, UserStatus};
use crate::state_helpers;

/// Handle a DHT value change event from a watched friend record.
///
/// Called by the Veilid dispatch loop when a friend's DHT record changes.
/// Subkey mapping:
///   2 = status enum
///   4 = game info
///   6 = route blob
pub fn handle_value_change(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    dht_key: &str,
    subkeys: &[u32],
    value: &[u8],
) {
    // Find which friend this DHT key belongs to via the DHT manager mapping
    let friend_key = state_helpers::friend_for_dht_key(state, dht_key);

    let Some(friend_key) = friend_key else {
        tracing::debug!(dht_key, "value change for unknown DHT key");
        // Still process - might be a community record
        handle_community_value_change(state, dht_key, subkeys, value);
        return;
    };

    for &subkey in subkeys {
        match subkey {
            2 => handle_status_change(app_handle, state, &friend_key, value),
            4 => handle_game_change(app_handle, state, &friend_key, value),
            6 => handle_route_change(state, &friend_key, value),
            _ => tracing::trace!(subkey, "unhandled presence subkey change"),
        }
    }
}

/// Handle a friend's status change (subkey 2).
fn handle_status_change(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    friend_key: &str,
    value: &[u8],
) {
    let Some(mut status) = parse_status(value) else {
        return;
    };

    // Override non-offline status to offline if timestamp is stale
    if status != UserStatus::Offline {
        if let Some(ts) = parse_status_timestamp(value) {
            let now = db::timestamp_now();
            if now - ts > STALE_PRESENCE_THRESHOLD_MS {
                tracing::info!(
                    friend = %friend_key, age_ms = now - ts,
                    "stale presence — treating as offline"
                );
                status = UserStatus::Offline;
            }
        }
    }

    // Store heartbeat timestamp on friend state
    if let Some(ts) = parse_status_timestamp(value) {
        let mut friends = state.friends.write();
        if let Some(friend) = friends.get_mut(friend_key) {
            friend.last_heartbeat_at = Some(ts);
        }
    }

    // Only emit presence events for accepted friends (privacy: hide status from pending)
    let is_accepted = state_helpers::is_friend_accepted(state, friend_key);

    let event = if status == UserStatus::Offline {
        let now = db::timestamp_now();
        {
            let mut friends = state.friends.write();
            if let Some(friend) = friends.get_mut(friend_key) {
                friend.status = UserStatus::Offline;
                friend.last_seen_at = Some(now);
            }
        }

        let pool: tauri::State<'_, DbPool> = app_handle.state();
        crate::friend_repo::fire_update_last_seen_at(state, pool.inner(), friend_key, now);

        PresenceEvent::FriendOffline {
            public_key: friend_key.to_string(),
        }
    } else {
        let was_offline = {
            let mut friends = state.friends.write();
            if let Some(friend) = friends.get_mut(friend_key) {
                let was = friend.status == UserStatus::Offline;
                friend.status = status;
                was
            } else {
                false
            }
        };

        if was_offline && is_accepted {
            let online_event = PresenceEvent::FriendOnline {
                public_key: friend_key.to_string(),
            };
            let _ = app_handle.emit("presence-event", &online_event);
        }

        PresenceEvent::StatusChanged {
            public_key: friend_key.to_string(),
            status: format!("{status:?}").to_lowercase(),
            status_message: None,
        }
    };
    if is_accepted {
        let _ = app_handle.emit("presence-event", &event);
    }
}

/// Handle a friend's game info change (subkey 4).
fn handle_game_change(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    friend_key: &str,
    value: &[u8],
) {
    let game_info = parse_game_info(value);
    {
        let mut friends = state.friends.write();
        if let Some(friend) = friends.get_mut(friend_key) {
            friend.game_info.clone_from(&game_info);
        }
    }
    // Only emit game events for accepted friends (privacy: hide from pending)
    if !state_helpers::is_friend_accepted(state, friend_key) {
        return;
    }
    let event = PresenceEvent::GameChanged {
        public_key: friend_key.to_string(),
        game_name: game_info.as_ref().map(|g| g.game_name.clone()),
        game_id: game_info.as_ref().map(|g| g.game_id),
        elapsed_seconds: game_info.as_ref().map(|g| g.elapsed_seconds),
        server_address: game_info.as_ref().and_then(|g| g.server_address.clone()),
    };
    let _ = app_handle.emit("presence-event", &event);
}

/// Handle a friend's route blob change (subkey 6).
fn handle_route_change(state: &Arc<AppState>, friend_key: &str, value: &[u8]) {
    tracing::debug!(friend = %friend_key, "friend route blob updated");
    if !value.is_empty() {
        if let Some(api) = state_helpers::veilid_api(state) {
            let mut dht_mgr = state.dht_manager.write();
            if let Some(mgr) = dht_mgr.as_mut() {
                mgr.manager.cache_route(&api, friend_key, value.to_vec());
            }
        }
    }
}

/// Handle value changes for community DHT records.
///
/// When a watched community DHT record changes, we re-read the affected subkeys
/// from DHT to get the latest values and update local state accordingly.
fn handle_community_value_change(
    state: &Arc<AppState>,
    dht_key: &str,
    subkeys: &[u32],
    value: &[u8],
) {
    let community_id = {
        let communities = state.communities.read();
        communities
            .values()
            .find(|c| c.governance_key.as_deref() == Some(dht_key))
            .map(|c| c.id.clone())
    };

    let Some(community_id) = community_id else {
        tracing::trace!(dht_key, "value change for unknown community DHT key");
        return;
    };
    tracing::trace!(
        community = %community_id,
        dht_key,
        subkeys = subkeys.len(),
        value_len = value.len(),
        "ignoring manifest-shaped community value-change handler in forward-only governance model"
    );
}

/// Start watching a friend's DHT record for presence updates.
pub async fn watch_friend(
    state: &Arc<AppState>,
    friend_key: &str,
    dht_record_key: &str,
) -> Result<(), String> {
    // Register the DHT key → friend mapping so handle_value_change can resolve it
    {
        let mut dht_mgr = state.dht_manager.write();
        if let Some(mgr) = dht_mgr.as_mut() {
            mgr.register_friend_dht_key(dht_record_key.to_string(), friend_key.to_string());
        }
    }

    // Also store the dht_record_key on the FriendState
    {
        let mut friends = state.friends.write();
        if let Some(friend) = friends.get_mut(friend_key) {
            friend.dht_record_key = Some(dht_record_key.to_string());
        }
    }

    // Clone routing_context (Arc-based, cheap) — must not hold parking_lot lock across .await
    let routing_context = {
        let node = state.node.read();
        node.as_ref().map(|nh| nh.routing_context.clone())
    };
    let Some(routing_context) = routing_context else {
        return Ok(());
    };

    // Parse the record key
    let record_key: veilid_core::RecordKey = dht_record_key
        .parse()
        .map_err(|e| format!("invalid DHT key: {e}"))?;

    // Open the record
    if let Err(e) = routing_context
        .open_dht_record(record_key.clone(), None)
        .await
    {
        tracing::warn!(error = %e, dht_key = %dht_record_key, "failed to open DHT record for watching");
        return Ok(()); // Non-fatal: friend will be synced on next interval
    }
    // Track opened record for cleanup on shutdown
    {
        let mut dht_mgr = state.dht_manager.write();
        if let Some(mgr) = dht_mgr.as_mut() {
            mgr.track_open_record(dht_record_key.to_string());
        }
    }

    // Watch presence subkeys (status=2, game=4, route=6)
    let subkey_range: veilid_core::ValueSubkeyRangeSet = [2u32, 4, 6].into_iter().collect();
    match routing_context
        .watch_dht_values(record_key, Some(subkey_range), None, None)
        .await
    {
        Ok(active) => {
            if active {
                tracing::info!(friend = %friend_key, dht_key = %dht_record_key, "watching friend presence");
                // Watch succeeded — remove from unwatched set if previously there
                state.unwatched_friends.write().remove(friend_key);
            } else {
                // Per Veilid GitLab #377: watch_dht_values returning false means
                // the watch could not be established. We must poll as fallback.
                tracing::warn!(
                    friend = %friend_key, dht_key = %dht_record_key,
                    "watch_dht_values returned false — adding to poll fallback set"
                );
                state
                    .unwatched_friends
                    .write()
                    .insert(friend_key.to_string());
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, friend = %friend_key, "failed to watch friend presence — adding to poll fallback set");
            state
                .unwatched_friends
                .write()
                .insert(friend_key.to_string());
        }
    }

    Ok(())
}

/// Publish our own status to DHT profile subkey 2.
pub async fn publish_status(state: &Arc<AppState>, status: UserStatus) -> Result<(), String> {
    let status_byte = match status {
        UserStatus::Online => 0u8,
        UserStatus::Away => 1,
        UserStatus::Busy => 2,
        // Invisible publishes as offline (3) so others see us as offline
        UserStatus::Offline | UserStatus::Invisible => 3,
    };

    // Get our profile DHT key, owner keypair, and routing context (brief lock, clone out)
    let (profile_key, owner_keypair, routing_context) = {
        let node = state.node.read();
        match node.as_ref() {
            Some(nh) => (
                nh.profile_dht_key.clone(),
                nh.profile_owner_keypair.clone(),
                Some(nh.routing_context.clone()),
            ),
            None => (None, None, None),
        }
    };

    let Some(profile_key) = profile_key else {
        tracing::warn!(status = ?status, "publish_status: no profile DHT key — node may not be ready");
        return Err("no profile DHT key".to_string());
    };
    let Some(routing_context) = routing_context else {
        tracing::warn!(status = ?status, "publish_status: no routing context");
        return Err("no routing context".to_string());
    };

    tracing::info!(
        status = ?status,
        has_owner_keypair = owner_keypair.is_some(),
        profile_key = %profile_key,
        "publish_status: writing to DHT"
    );

    let record_key: veilid_core::RecordKey = profile_key
        .parse()
        .map_err(|e| format!("invalid profile key: {e}"))?;

    // Ensure the record is open with write access before writing.
    // Re-opening an already-open record is a no-op in Veilid.
    if let Err(e) = routing_context
        .open_dht_record(record_key.clone(), owner_keypair)
        .await
    {
        tracing::warn!(error = %e, "publish_status: failed to open profile record");
        return Err(format!("failed to open profile record: {e}"));
    }

    let timestamp = crate::db::timestamp_now();
    let mut payload = Vec::with_capacity(9);
    payload.push(status_byte);
    payload.extend_from_slice(&timestamp.to_be_bytes());

    routing_context
        .set_dht_value(record_key, 2, payload, None)
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, status = ?status, "publish_status: set_dht_value failed");
            format!("failed to publish status to DHT: {e}")
        })?;

    tracing::info!(status = ?status, "published status to DHT");
    Ok(())
}

/// 2.5x the 60s heartbeat — allows one missed heartbeat + jitter before marking offline.
pub const STALE_PRESENCE_THRESHOLD_MS: i64 = 150 * 1000;

/// Parse status byte from a status payload.
///
/// Accepts both the legacy 1-byte format `[status]` and the new 9-byte
/// format `[status, timestamp_be]`. The timestamp is extracted separately
/// via `parse_status_timestamp`.
pub fn parse_status(data: &[u8]) -> Option<UserStatus> {
    if data.is_empty() {
        return None;
    }
    Some(match data[0] {
        0 => UserStatus::Online,
        1 => UserStatus::Away,
        2 => UserStatus::Busy,
        _ => UserStatus::Offline,
    })
}

/// Extract the heartbeat timestamp from the 9-byte status payload.
pub fn parse_status_timestamp(data: &[u8]) -> Option<i64> {
    if data.len() < 9 {
        return None;
    }
    let bytes: [u8; 8] = data[1..9].try_into().ok()?;
    Some(i64::from_be_bytes(bytes))
}

fn parse_game_info(data: &[u8]) -> Option<GameInfoState> {
    // TODO: Deserialize Cap'n Proto GameStatus from data
    if data.is_empty() {
        return None;
    }
    // Placeholder: try JSON deserialization
    serde_json::from_slice(data).ok()
}

/// Periodically re-publish our current status with a fresh timestamp.
///
/// This serves as a keepalive: friends detect stale timestamps and infer
/// that we've gone offline (crashed without publishing Offline).
/// Runs every 60 seconds to keep friends' stale detection fresh.
pub async fn start_heartbeat_loop(
    state: Arc<AppState>,
    mut shutdown_rx: tokio::sync::mpsc::Receiver<()>,
) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
    interval.tick().await; // Skip immediate first tick

    loop {
        tokio::select! {
            _ = interval.tick() => {
                let status = match state_helpers::identity_status(&state) {
                    Some(s) if s != UserStatus::Offline => s,
                    _ => continue, // Not logged in or already offline — skip
                };
                if let Err(e) = publish_status(&state, status).await {
                    tracing::debug!(error = %e, "heartbeat publish failed");
                }
            }
            _ = shutdown_rx.recv() => {
                tracing::debug!("presence heartbeat loop shutting down");
                break;
            }
        }
    }
}
