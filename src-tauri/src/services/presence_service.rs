use std::sync::Arc;

use tauri::{Emitter, Manager};

use crate::channels::{NotificationEvent, PresenceEvent};
use crate::db::{self, DbPool};
use crate::state::{AppState, GameInfoState, UserStatus};

/// Handle a DHT value change event from a watched friend record.
///
/// Called by the Veilid dispatch loop when a friend's DHT record changes.
/// Subkey mapping:
///   2 = status enum
///   4 = game info
///   6 = route blob
pub async fn handle_value_change(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    dht_key: &str,
    subkeys: &[u32],
    value: &[u8],
) {
    // Find which friend this DHT key belongs to via the DHT manager mapping
    let friend_key = {
        let dht_mgr = state.dht_manager.read();
        dht_mgr
            .as_ref()
            .and_then(|mgr| mgr.friend_for_dht_key(dht_key).cloned())
    };

    let Some(friend_key) = friend_key else {
        tracing::debug!(dht_key, "value change for unknown DHT key");
        // Still process - might be a community record
        handle_community_value_change(app_handle, state, dht_key, subkeys, value).await;
        return;
    };

    for &subkey in subkeys {
        match subkey {
            // Status enum changed
            2 => {
                if let Some(status) = parse_status(value) {
                    let event = if status == UserStatus::Offline {
                        // Record the timestamp when this friend went offline
                        let now = db::timestamp_now();
                        {
                            let mut friends = state.friends.write();
                            if let Some(friend) = friends.get_mut(&friend_key) {
                                friend.status = UserStatus::Offline;
                                friend.last_seen_at = Some(now);
                            }
                        }

                        // Persist last_seen_at to the database
                        let pool: tauri::State<'_, DbPool> = app_handle.state();
                        let pool_clone = pool.inner().clone();
                        let fk = friend_key.clone();
                        let ok = state
                            .identity
                            .read()
                            .as_ref()
                            .map(|id| id.public_key.clone())
                            .unwrap_or_default();
                        drop(tokio::task::spawn_blocking(move || {
                            if let Ok(conn) = pool_clone.lock() {
                                let _ = conn.execute(
                                    "UPDATE friends SET last_seen_at = ?1 WHERE owner_key = ?2 AND public_key = ?3",
                                    rusqlite::params![now, ok, fk],
                                );
                            }
                        }));

                        PresenceEvent::FriendOffline {
                            public_key: friend_key.clone(),
                        }
                    } else {
                        // Update friend state and check if coming online
                        let was_offline = {
                            let mut friends = state.friends.write();
                            if let Some(friend) = friends.get_mut(&friend_key) {
                                let was = friend.status == UserStatus::Offline;
                                friend.status = status;
                                was
                            } else {
                                false
                            }
                        };

                        // Emit FriendOnline if transitioning from offline
                        if was_offline {
                            let online_event = PresenceEvent::FriendOnline {
                                public_key: friend_key.clone(),
                            };
                            let _ = app_handle.emit("presence-event", &online_event);
                        }

                        PresenceEvent::StatusChanged {
                            public_key: friend_key.clone(),
                            status: format!("{status:?}").to_lowercase(),
                            status_message: None,
                        }
                    };
                    let _ = app_handle.emit("presence-event", &event);
                }
            }
            // Game info changed
            4 => {
                let game_info = parse_game_info(value);
                {
                    let mut friends = state.friends.write();
                    if let Some(friend) = friends.get_mut(&friend_key) {
                        friend.game_info.clone_from(&game_info);
                    }
                }
                let event = PresenceEvent::GameChanged {
                    public_key: friend_key.clone(),
                    game_name: game_info.as_ref().map(|g| g.game_name.clone()),
                    game_id: game_info.as_ref().map(|g| g.game_id),
                    elapsed_seconds: game_info.as_ref().map(|g| g.elapsed_seconds),
                };
                let _ = app_handle.emit("presence-event", &event);
            }
            // Route blob changed (we can now send messages to them)
            6 => {
                tracing::debug!(friend = %friend_key, "friend route blob updated");
                // Cache the new route blob for message sending
                if !value.is_empty() {
                    let api = {
                        let node = state.node.read();
                        node.as_ref().map(|nh| nh.api.clone())
                    };
                    if let Some(api) = api {
                        let mut dht_mgr = state.dht_manager.write();
                        if let Some(mgr) = dht_mgr.as_mut() {
                            mgr.manager.cache_route(&api, &friend_key, value.to_vec());
                        }
                    }
                }
            }
            _ => {
                tracing::trace!(subkey, "unhandled presence subkey change");
            }
        }
    }
}

/// Handle value changes for community DHT records.
///
/// When a watched community DHT record changes, we re-read the affected subkeys
/// from DHT to get the latest values and update local state accordingly.
async fn handle_community_value_change(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    dht_key: &str,
    subkeys: &[u32],
    value: &[u8],
) {
    // Find which community this DHT key belongs to
    let community_id = {
        let communities = state.communities.read();
        communities
            .values()
            .find(|c| c.dht_record_key.as_deref() == Some(dht_key))
            .map(|c| c.id.clone())
    };

    let Some(community_id) = community_id else {
        tracing::trace!(dht_key, "value change for unknown community DHT key");
        return;
    };

    // Clone routing context out before any .await (parking_lot guards are !Send)
    let routing_context = {
        let node = state.node.read();
        node.as_ref()
            .filter(|nh| nh.is_attached)
            .map(|nh| nh.routing_context.clone())
    };

    // Use a DHTManager to re-read subkeys that may have stale inline values
    let mgr = routing_context.map(rekindle_protocol::dht::DHTManager::new);

    for &subkey in subkeys {
        match subkey {
            0 => handle_community_metadata_change(state, &community_id, mgr.as_ref(), dht_key, value).await,
            1 => handle_community_channel_list_change(state, &community_id, mgr.as_ref(), dht_key).await,
            2 => handle_community_member_list_change(app_handle, state, &community_id, mgr.as_ref(), dht_key).await,
            _ => {
                tracing::trace!(community = %community_id, subkey, "unhandled community subkey");
            }
        }
    }

    // Notify frontend about community update via typed notification channel
    let notification = NotificationEvent::SystemAlert {
        title: "Community Update".to_string(),
        body: format!("Community {community_id} has been updated"),
    };
    let _ = app_handle.emit("notification-event", &notification);
}

/// Process a community metadata change (subkey 0: name, description, icon).
///
/// Uses the inline `value` first; falls back to a DHT read if needed.
async fn handle_community_metadata_change(
    state: &Arc<AppState>,
    community_id: &str,
    mgr: Option<&rekindle_protocol::dht::DHTManager>,
    dht_key: &str,
    value: &[u8],
) {
    let metadata_bytes = if !value.is_empty() {
        Some(value.to_vec())
    } else if let Some(mgr) = mgr {
        mgr.get_value(dht_key, 0).await.ok().flatten()
    } else {
        None
    };

    if let Some(data) = metadata_bytes {
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
    tracing::debug!(community = %community_id, "community metadata updated");
}

/// Process a community channel list change (subkey 1).
///
/// Re-reads the channel list from DHT and updates local state.
async fn handle_community_channel_list_change(
    state: &Arc<AppState>,
    community_id: &str,
    mgr: Option<&rekindle_protocol::dht::DHTManager>,
    dht_key: &str,
) {
    let channel_data = if let Some(mgr) = mgr {
        mgr.get_value(dht_key, 1).await.ok().flatten()
    } else {
        None
    };

    if let Some(data) = channel_data {
        if let Ok(channel_list) = serde_json::from_slice::<Vec<serde_json::Value>>(&data) {
            let channels: Vec<crate::state::ChannelInfo> = channel_list
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
                .collect();

            let mut communities = state.communities.write();
            if let Some(community) = communities.get_mut(community_id) {
                community.channels = channels;
            }
        }
    }
    tracing::debug!(community = %community_id, "community channel list updated");
}

/// Process a community member list change (subkey 2).
///
/// Re-reads the member list from DHT and persists it to the local database.
async fn handle_community_member_list_change(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    mgr: Option<&rekindle_protocol::dht::DHTManager>,
    dht_key: &str,
) {
    let member_data = if let Some(mgr) = mgr {
        mgr.get_value(dht_key, 2).await.ok().flatten()
    } else {
        None
    };

    let Some(data) = member_data else {
        tracing::debug!(community = %community_id, "community member list updated (no data)");
        return;
    };

    // Parse member list JSON: [{publicKey, displayName, role, joinedAt}, ...]
    let Ok(member_list) = serde_json::from_slice::<Vec<serde_json::Value>>(&data) else {
        return;
    };

    let member_count = member_list.len();
    let pool: tauri::State<'_, DbPool> = app_handle.state();
    let pool = pool.inner().clone();
    let cid = community_id.to_string();
    let owner_key = state
        .identity
        .read()
        .as_ref()
        .map(|id| id.public_key.clone())
        .unwrap_or_default();
    if let Err(e) = tokio::task::spawn_blocking(move || {
        let conn = pool.lock().map_err(|e| e.to_string())?;
        for member in &member_list {
            let Some(pk) = member.get("publicKey").and_then(|v| v.as_str()) else {
                continue;
            };
            let dn = member
                .get("displayName")
                .and_then(|v| v.as_str())
                .unwrap_or(pk);
            let role = member
                .get("role")
                .and_then(|v| v.as_str())
                .unwrap_or("member");
            let joined_at = member
                .get("joinedAt")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or_else(crate::db::timestamp_now);
            conn.execute(
                "INSERT OR REPLACE INTO community_members \
                 (owner_key, community_id, public_key, display_name, role, joined_at) \
                 VALUES (?, ?, ?, ?, ?, ?)",
                rusqlite::params![owner_key, cid, pk, dn, role, joined_at],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok::<_, String>(())
    })
    .await
    .unwrap_or_else(|e| Err(e.to_string()))
    {
        tracing::warn!(
            community = %community_id,
            error = %e,
            "failed to persist community member list"
        );
    }
    tracing::debug!(
        community = %community_id,
        members = member_count,
        "community member list updated from DHT"
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
    if let Err(e) = routing_context.open_dht_record(record_key.clone(), None).await {
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
            tracing::info!(friend = %friend_key, dht_key = %dht_record_key, active, "watching friend presence");
        }
        Err(e) => {
            tracing::warn!(error = %e, friend = %friend_key, "failed to watch friend presence");
        }
    }

    Ok(())
}

/// Publish our own status to DHT profile subkey 2.
pub async fn publish_status(
    state: &Arc<AppState>,
    status: UserStatus,
) -> Result<(), String> {
    let status_byte = match status {
        UserStatus::Online => 0u8,
        UserStatus::Away => 1,
        UserStatus::Busy => 2,
        UserStatus::Offline => 3,
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
        tracing::debug!(status = ?status, "no profile DHT key yet, skipping status publish");
        return Ok(());
    };
    let Some(routing_context) = routing_context else {
        return Ok(());
    };

    let record_key: veilid_core::RecordKey = profile_key
        .parse()
        .map_err(|e| format!("invalid profile key: {e}"))?;

    // Ensure the record is open with write access before writing.
    if let Err(e) = routing_context
        .open_dht_record(record_key.clone(), owner_keypair)
        .await
    {
        tracing::warn!(error = %e, "failed to open profile record for status publish");
        return Ok(());
    }

    match routing_context
        .set_dht_value(record_key, 2, vec![status_byte], None)
        .await
    {
        Ok(_) => {
            tracing::info!(status = ?status, "published status to DHT");
        }
        Err(e) => {
            tracing::warn!(error = %e, "failed to publish status to DHT");
        }
    }

    Ok(())
}

fn parse_status(data: &[u8]) -> Option<UserStatus> {
    data.first().map(|b| match b {
        0 => UserStatus::Online,
        1 => UserStatus::Away,
        2 => UserStatus::Busy,
        _ => UserStatus::Offline,
    })
}

fn parse_game_info(data: &[u8]) -> Option<GameInfoState> {
    // TODO: Deserialize Cap'n Proto GameStatus from data
    if data.is_empty() {
        return None;
    }
    // Placeholder: try JSON deserialization
    serde_json::from_slice(data).ok()
}
