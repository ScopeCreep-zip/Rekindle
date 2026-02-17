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
    let Some(status) = parse_status(value) else {
        return;
    };

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
        let pool_clone = pool.inner().clone();
        let fk = friend_key.to_string();
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

        if was_offline {
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
    let _ = app_handle.emit("presence-event", &event);
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
    let event = PresenceEvent::GameChanged {
        public_key: friend_key.to_string(),
        game_name: game_info.as_ref().map(|g| g.game_name.clone()),
        game_id: game_info.as_ref().map(|g| g.game_id),
        elapsed_seconds: game_info.as_ref().map(|g| g.elapsed_seconds),
    };
    let _ = app_handle.emit("presence-event", &event);
}

/// Handle a friend's route blob change (subkey 6).
fn handle_route_change(
    state: &Arc<AppState>,
    friend_key: &str,
    value: &[u8],
) {
    tracing::debug!(friend = %friend_key, "friend route blob updated");
    if !value.is_empty() {
        let api = {
            let node = state.node.read();
            node.as_ref().map(|nh| nh.api.clone())
        };
        if let Some(api) = api {
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
            0 => handle_community_metadata_change(app_handle, state, &community_id, mgr.as_ref(), dht_key, value).await,
            1 => handle_community_channel_list_change(app_handle, state, &community_id, mgr.as_ref(), dht_key).await,
            2 => handle_community_member_list_change(app_handle, state, &community_id, mgr.as_ref(), dht_key).await,
            3 => handle_community_roles_change(app_handle, state, &community_id, mgr.as_ref(), dht_key).await,
            5 => handle_community_mek_change(app_handle, state, &community_id).await,
            6 => handle_community_route_change(app_handle, state, &community_id, mgr.as_ref(), dht_key).await,
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
    app_handle: &tauri::AppHandle,
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
            let (new_name, new_desc) = {
                let mut communities = state.communities.write();
                if let Some(community) = communities.get_mut(community_id) {
                    if let Some(name) = metadata.get("name").and_then(|v| v.as_str()) {
                        community.name = name.to_string();
                    }
                    if let Some(desc) = metadata.get("description").and_then(|v| v.as_str()) {
                        community.description = Some(desc.to_string());
                    }
                    (Some(community.name.clone()), community.description.clone())
                } else {
                    (None, None)
                }
            };
            // Persist to SQLite
            if let Some(name) = new_name {
                let owner_key = state.identity.read().as_ref().map(|id| id.public_key.clone()).unwrap_or_default();
                let pool: tauri::State<'_, DbPool> = app_handle.state();
                let pool = pool.inner().clone();
                let cid = community_id.to_string();
                let desc = new_desc;
                let _ = tokio::task::spawn_blocking(move || {
                    let conn = pool.lock().map_err(|e| e.to_string())?;
                    conn.execute(
                        "UPDATE communities SET name = ?, description = ? WHERE owner_key = ? AND id = ?",
                        rusqlite::params![name, desc, owner_key, cid],
                    ).map_err(|e| e.to_string())?;
                    Ok::<_, String>(())
                }).await;
            }
        }
    }
    tracing::debug!(community = %community_id, "community metadata updated");
}

/// Process a community channel list change (subkey 1).
///
/// Re-reads the channel list from DHT and updates local state.
async fn handle_community_channel_list_change(
    app_handle: &tauri::AppHandle,
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
        // Support both wrapped format { channels: [...], lastRefreshed } and bare array [...]
        let channel_list_opt: Option<Vec<serde_json::Value>> =
            serde_json::from_slice::<serde_json::Value>(&data)
                .ok()
                .and_then(|v| {
                    if let Some(obj) = v.as_object() {
                        obj.get("channels").and_then(|c| c.as_array().cloned())
                    } else {
                        v.as_array().cloned()
                    }
                });
        if let Some(channel_list) = channel_list_opt {
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

            {
                let mut communities = state.communities.write();
                if let Some(community) = communities.get_mut(community_id) {
                    community.channels.clone_from(&channels);
                }
            }

            // Persist to SQLite: DELETE + INSERT
            let owner_key = state.identity.read().as_ref().map(|id| id.public_key.clone()).unwrap_or_default();
            let pool: tauri::State<'_, DbPool> = app_handle.state();
            let pool = pool.inner().clone();
            let cid = community_id.to_string();
            let _ = tokio::task::spawn_blocking(move || {
                let conn = pool.lock().map_err(|e| e.to_string())?;
                conn.execute(
                    "DELETE FROM channels WHERE owner_key = ? AND community_id = ?",
                    rusqlite::params![owner_key, cid],
                ).map_err(|e| e.to_string())?;
                for ch in &channels {
                    let ch_type = match ch.channel_type {
                        crate::state::ChannelType::Text => "text",
                        crate::state::ChannelType::Voice => "voice",
                    };
                    conn.execute(
                        "INSERT OR IGNORE INTO channels (owner_key, id, community_id, name, channel_type) VALUES (?, ?, ?, ?, ?)",
                        rusqlite::params![owner_key, ch.id, cid, ch.name, ch_type],
                    ).map_err(|e| e.to_string())?;
                }
                Ok::<_, String>(())
            }).await;
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

    // Parse member list JSON: wrapped { members: [...], lastRefreshed } or bare [...]
    let member_list: Vec<serde_json::Value> = match serde_json::from_slice::<serde_json::Value>(&data) {
        Ok(v) => {
            if let Some(obj) = v.as_object() {
                obj.get("members").and_then(|m| m.as_array().cloned()).unwrap_or_default()
            } else {
                v.as_array().cloned().unwrap_or_default()
            }
        }
        Err(_) => return,
    };
    if member_list.is_empty() && data.len() > 2 {
        // Data was non-trivial but couldn't parse — skip silently
        return;
    }

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
            let Some(pk) = member.get("pseudonymKey").and_then(|v| v.as_str()) else {
                continue;
            };
            let dn = member
                .get("displayName")
                .and_then(|v| v.as_str())
                .unwrap_or(pk);
            let role_ids = member
                .get("roleIds")
                .map_or_else(|| "[0,1]".to_string(), std::string::ToString::to_string);
            let joined_at = member
                .get("joinedAt")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or_else(crate::db::timestamp_now);
            conn.execute(
                "INSERT OR REPLACE INTO community_members \
                 (owner_key, community_id, pseudonym_key, display_name, role_ids, joined_at) \
                 VALUES (?, ?, ?, ?, ?, ?)",
                rusqlite::params![owner_key, cid, pk, dn, role_ids, joined_at],
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

/// Handle DHT subkey 3 change: community role definitions updated.
///
/// Reads the updated role list from DHT, updates in-memory `CommunityState.roles`,
/// persists to the `community_roles` table, and emits a `RolesChanged` event.
async fn handle_community_roles_change(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    mgr: Option<&rekindle_protocol::dht::DHTManager>,
    dht_key: &str,
) {
    let Some(mgr) = mgr else {
        tracing::warn!(community = %community_id, "cannot fetch role updates — not attached");
        return;
    };

    let roles = match rekindle_protocol::dht::community::read_roles(mgr, dht_key).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(community = %community_id, error = %e, "failed to read roles from DHT");
            return;
        }
    };

    // Update in-memory state
    let role_defs: Vec<crate::state::RoleDefinition> = roles
        .iter()
        .map(|r| crate::state::RoleDefinition {
            id: r.id,
            name: r.name.clone(),
            color: r.color,
            permissions: r.permissions,
            position: r.position,
            hoist: r.hoist,
            mentionable: r.mentionable,
        })
        .collect();
    {
        let mut communities = state.communities.write();
        if let Some(c) = communities.get_mut(community_id) {
            c.roles.clone_from(&role_defs);
            c.my_role = Some(crate::state::display_role_name(&c.my_role_ids, &c.roles));
        }
    }

    // Persist to SQLite
    let owner_key = state
        .identity
        .read()
        .as_ref()
        .map(|id| id.public_key.clone())
        .unwrap_or_default();
    let pool: tauri::State<'_, DbPool> = app_handle.state();
    let pool = pool.inner().clone();
    let cid = community_id.to_string();
    let role_defs_for_db = role_defs.clone();
    let _ = tokio::task::spawn_blocking(move || {
        let conn = pool.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM community_roles WHERE owner_key = ? AND community_id = ?",
            rusqlite::params![owner_key, cid],
        )
        .map_err(|e| e.to_string())?;
        for r in &role_defs_for_db {
            conn.execute(
                "INSERT INTO community_roles \
                 (owner_key, community_id, role_id, name, color, permissions, position, hoist, mentionable) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    owner_key,
                    cid,
                    r.id,
                    r.name,
                    r.color,
                    r.permissions.cast_signed(),
                    r.position,
                    r.hoist,
                    r.mentionable,
                ],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok::<_, String>(())
    })
    .await;

    // Emit frontend event
    let event = crate::channels::CommunityEvent::RolesChanged {
        community_id: community_id.to_string(),
        roles: role_defs
            .iter()
            .map(|r| crate::channels::community_channel::RoleDto {
                id: r.id,
                name: r.name.clone(),
                color: r.color,
                permissions: r.permissions,
                position: r.position,
                hoist: r.hoist,
                mentionable: r.mentionable,
            })
            .collect(),
    };
    let _ = app_handle.emit("community-event", &event);

    tracing::info!(
        community = %community_id,
        role_count = role_defs.len(),
        "community roles updated from DHT"
    );
}

/// Handle DHT subkey 5 change: MEK bundles updated.
///
/// When the server publishes new MEK bundles (e.g., after rotation), re-fetch from server
/// via the existing `RequestMEK` RPC so we get the latest key.
async fn handle_community_mek_change(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
) {
    tracing::info!(community = %community_id, "MEK bundles updated in DHT — fetching new MEK from server");
    super::veilid_service::fetch_mek_from_server(app_handle, state, community_id).await;
}

/// Handle DHT subkey 6 change: server route blob updated.
///
/// The community server's private route has changed (e.g., route died and was re-allocated).
/// Read the new route blob from DHT, update in-memory state, and persist to `SQLite`.
async fn handle_community_route_change(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    mgr: Option<&rekindle_protocol::dht::DHTManager>,
    dht_key: &str,
) {
    let Some(mgr) = mgr else {
        tracing::warn!(community = %community_id, "cannot fetch updated server route — not attached");
        return;
    };

    match mgr.get_value(dht_key, rekindle_protocol::dht::community::SUBKEY_SERVER_ROUTE).await {
        Ok(Some(route_blob)) => {
            {
                let mut communities = state.communities.write();
                if let Some(community) = communities.get_mut(community_id) {
                    community.server_route_blob = Some(route_blob.clone());
                }
            }

            // Persist to SQLite so the route survives logout/restart
            crate::commands::community::persist_server_route_blob(
                app_handle, community_id, &route_blob,
            )
            .await;

            tracing::info!(community = %community_id, "updated server route blob from DHT");
        }
        Ok(None) => {
            tracing::debug!(community = %community_id, "server route blob is empty in DHT");
        }
        Err(e) => {
            tracing::warn!(error = %e, community = %community_id, "failed to read server route from DHT");
        }
    }
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
                state.unwatched_friends.write().insert(friend_key.to_string());
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, friend = %friend_key, "failed to watch friend presence — adding to poll fallback set");
            state.unwatched_friends.write().insert(friend_key.to_string());
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

    routing_context
        .set_dht_value(record_key, 2, vec![status_byte], None)
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, status = ?status, "publish_status: set_dht_value failed");
            format!("failed to publish status to DHT: {e}")
        })?;

    tracing::info!(status = ?status, "published status to DHT");
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
