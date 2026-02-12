use rekindle_protocol::dht::DHTManager;
use rusqlite::OptionalExtension;
use serde::Serialize;
use tauri::{Emitter, State};

use crate::channels::ChatEvent;
use crate::commands::auth::current_owner_key;
use crate::commands::chat::Message;
use crate::db::{self, DbPool};
use crate::services;
use crate::state::{ChannelType, SharedState};

/// A community member for the frontend.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberDto {
    pub public_key: String,
    pub display_name: String,
    pub role: String,
    pub status: String,
}

/// A community summary for the frontend.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityInfo {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub channel_count: usize,
    pub my_role: Option<String>,
}

/// Get the list of joined communities.
#[tauri::command]
pub async fn get_communities(
    state: State<'_, SharedState>,
) -> Result<Vec<CommunityInfo>, String> {
    let communities = state.communities.read();
    let list = communities
        .values()
        .map(|c| CommunityInfo {
            id: c.id.clone(),
            name: c.name.clone(),
            description: c.description.clone(),
            channel_count: c.channels.len(),
            my_role: c.my_role.clone(),
        })
        .collect();
    Ok(list)
}

/// Channel info for the frontend.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelInfoDto {
    pub id: String,
    pub name: String,
    pub channel_type: String,
    pub unread_count: u32,
}

/// Full community detail with channels for the frontend.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityDetail {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub channels: Vec<ChannelInfoDto>,
    pub my_role: Option<String>,
}

/// Get all joined communities with full channel details.
#[tauri::command]
pub async fn get_community_details(
    state: State<'_, SharedState>,
) -> Result<Vec<CommunityDetail>, String> {
    let communities = state.communities.read();
    let list = communities
        .values()
        .map(|c| CommunityDetail {
            id: c.id.clone(),
            name: c.name.clone(),
            description: c.description.clone(),
            channels: c
                .channels
                .iter()
                .map(|ch| ChannelInfoDto {
                    id: ch.id.clone(),
                    name: ch.name.clone(),
                    channel_type: match ch.channel_type {
                        ChannelType::Text => "text".to_string(),
                        ChannelType::Voice => "voice".to_string(),
                    },
                    unread_count: ch.unread_count,
                })
                .collect(),
            my_role: c.my_role.clone(),
        })
        .collect();
    Ok(list)
}

/// Create a new community and store it in `AppState` + `SQLite`.
#[tauri::command]
pub async fn create_community(
    name: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<String, String> {
    let owner_key = current_owner_key(state.inner())?;
    let community_id =
        services::community_service::create_community(state.inner(), &name).await?;

    // Read back the community to get default channel info
    let community = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .cloned()
            .ok_or("community not found after creation")?
    };

    // Read creator identity outside spawn_blocking (parking_lot guard is !Send)
    let creator_key = owner_key.clone();
    let creator_name = state
        .identity
        .read()
        .as_ref()
        .map(|id| id.display_name.clone())
        .unwrap_or_default();

    let now = db::timestamp_now();
    let pool = pool.inner().clone();
    let community_id_clone = community_id.clone();
    let name_clone = name.clone();
    let dht_record_key = community.dht_record_key.clone();
    let ok = owner_key;
    tokio::task::spawn_blocking(move || {
        let conn = pool.lock().map_err(|e| e.to_string())?;

        conn.execute(
            "INSERT INTO communities (owner_key, id, name, my_role, joined_at, dht_record_key) VALUES (?, ?, ?, 'owner', ?, ?)",
            rusqlite::params![ok, community_id_clone, name_clone, now, dht_record_key],
        )
        .map_err(|e| e.to_string())?;

        // Insert the creator as the first member
        conn.execute(
            "INSERT INTO community_members (owner_key, community_id, public_key, display_name, role, joined_at) \
             VALUES (?, ?, ?, ?, 'owner', ?)",
            rusqlite::params![ok, community_id_clone, creator_key, creator_name, now],
        )
        .map_err(|e| e.to_string())?;

        // Insert default channels
        for channel in &community.channels {
            let ch_type = match channel.channel_type {
                ChannelType::Text => "text",
                ChannelType::Voice => "voice",
            };
            conn.execute(
                "INSERT INTO channels (owner_key, id, community_id, name, channel_type) VALUES (?, ?, ?, ?, ?)",
                rusqlite::params![ok, channel.id, community_id_clone, channel.name, ch_type],
            )
            .map_err(|e| e.to_string())?;
        }

        Ok::<_, String>(())
    })
    .await
    .map_err(|e| e.to_string())??;

    Ok(community_id)
}

/// Join an existing community by ID.
#[tauri::command]
pub async fn join_community(
    community_id: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let owner_key = current_owner_key(state.inner())?;
    services::community_service::join_community(state.inner(), &community_id).await?;

    let (name, dht_record_key) = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .map(|c| (c.name.clone(), c.dht_record_key.clone()))
            .unwrap_or_default()
    };

    // Read joiner identity outside spawn_blocking (parking_lot guard is !Send)
    let joiner_key = owner_key.clone();
    let joiner_name = state
        .identity
        .read()
        .as_ref()
        .map(|id| id.display_name.clone())
        .unwrap_or_default();

    let now = db::timestamp_now();
    let pool = pool.inner().clone();
    let community_id_clone = community_id.clone();
    let ok = owner_key;
    tokio::task::spawn_blocking(move || {
        let conn = pool.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT OR IGNORE INTO communities (owner_key, id, name, my_role, joined_at, dht_record_key) VALUES (?, ?, ?, 'member', ?, ?)",
            rusqlite::params![ok, community_id_clone, name, now, dht_record_key],
        )
        .map_err(|e| e.to_string())?;

        // Add ourselves to the community_members table
        conn.execute(
            "INSERT OR IGNORE INTO community_members (owner_key, community_id, public_key, display_name, role, joined_at) \
             VALUES (?, ?, ?, ?, 'member', ?)",
            rusqlite::params![ok, community_id_clone, joiner_key, joiner_name, now],
        )
        .map_err(|e| e.to_string())?;

        Ok::<_, String>(())
    })
    .await
    .map_err(|e| e.to_string())??;

    Ok(())
}

/// Create a new channel in a community.
#[tauri::command]
pub async fn create_channel(
    community_id: String,
    name: String,
    channel_type: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<String, String> {
    let owner_key = current_owner_key(state.inner())?;
    let channel_id = services::community_service::create_channel(
        state.inner(),
        &community_id,
        &name,
        &channel_type,
    )
    .await?;

    let pool = pool.inner().clone();
    let channel_id_clone = channel_id.clone();
    let community_id_clone = community_id.clone();
    let name_clone = name.clone();
    let channel_type_clone = channel_type.clone();
    tokio::task::spawn_blocking(move || {
        let conn = pool.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO channels (owner_key, id, community_id, name, channel_type) VALUES (?, ?, ?, ?, ?)",
            rusqlite::params![owner_key, channel_id_clone, community_id_clone, name_clone, channel_type_clone],
        )
        .map_err(|e| e.to_string())?;
        Ok::<_, String>(())
    })
    .await
    .map_err(|e| e.to_string())??;

    Ok(channel_id)
}

/// Community DHT subkey used for channel messages.
///
/// Subkey layout is defined in `community_service.rs`:
///   0 = metadata, 1 = channels, 2 = members, 3 = messages (append-only batch)
const SUBKEY_MESSAGES: u32 = 3;

/// Send a message in a community channel.
#[tauri::command]
pub async fn send_channel_message(
    channel_id: String,
    body: String,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let owner_key = current_owner_key(state.inner())?;
    let sender_key = owner_key.clone();

    let timestamp = db::timestamp_now();

    // --- Step 1: Store in SQLite (local persistence) ---
    let pool = pool.inner().clone();
    let channel_id_clone = channel_id.clone();
    let sender_key_clone = sender_key.clone();
    let body_clone = body.clone();
    let ok = owner_key;
    tokio::task::spawn_blocking(move || {
        let conn = pool.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO messages (owner_key, conversation_id, conversation_type, sender_key, body, timestamp, is_read) \
             VALUES (?, ?, 'channel', ?, ?, ?, 1)",
            rusqlite::params![ok, channel_id_clone, sender_key_clone, body_clone, timestamp],
        )
        .map_err(|e| e.to_string())?;
        Ok::<_, String>(())
    })
    .await
    .map_err(|e| e.to_string())??;

    // --- Step 2: Serialize message as JSON for DHT ---
    let message_json = serde_json::json!({
        "from": sender_key,
        "body": body,
        "timestamp": timestamp,
    });
    let json_bytes = serde_json::to_vec(&message_json)
        .map_err(|e| format!("failed to serialize channel message: {e}"))?;

    // --- Step 3: Encrypt with community MEK if available ---
    // MEK is not yet persisted to Stronghold (TODO in community_service),
    // so we send plaintext JSON with a warning for now.
    let payload = {
        // TODO: Look up MEK from Stronghold via VAULT_COMMUNITIES / mek_{community_id}
        // For now, we have no stored MEK handle, so we use plaintext.
        tracing::warn!(
            channel = %channel_id,
            "MEK not available (Stronghold integration pending) — sending plaintext channel message"
        );
        json_bytes
    };

    // --- Step 4: Write to community DHT record (subkey 3) ---
    // Find the community that owns this channel and extract its DHT record key.
    let dht_record_key = {
        let communities = state.communities.read();
        communities
            .values()
            .find(|c| c.channels.iter().any(|ch| ch.id == channel_id))
            .and_then(|c| c.dht_record_key.clone())
    };

    if let Some(dht_key) = dht_record_key {
        // Clone routing context out of the parking_lot lock before .await
        let routing_context = {
            let node = state.node.read();
            node.as_ref()
                .filter(|nh| nh.is_attached)
                .map(|nh| nh.routing_context.clone())
        };

        if let Some(rc) = routing_context {
            let mgr = DHTManager::new(rc);
            if let Err(e) = mgr.set_value(&dht_key, SUBKEY_MESSAGES, payload).await {
                tracing::warn!(
                    error = %e,
                    channel = %channel_id,
                    "failed to write channel message to DHT — stored locally only"
                );
            } else {
                tracing::debug!(
                    channel = %channel_id,
                    dht_key = %dht_key,
                    "channel message written to DHT subkey {SUBKEY_MESSAGES}"
                );
            }
        } else {
            tracing::debug!(
                channel = %channel_id,
                "node not attached — channel message stored locally only"
            );
        }
    } else {
        tracing::debug!(
            channel = %channel_id,
            "no DHT record for community — channel message stored locally only"
        );
    }

    // --- Step 5: Emit local echo to frontend ---
    let event = ChatEvent::MessageReceived {
        from: sender_key,
        body,
        timestamp: timestamp.cast_unsigned(),
        conversation_id: channel_id,
    };
    let _ = app.emit("chat-event", &event);

    tracing::info!("channel message sent");
    Ok(())
}

/// Leave a community and clean up local state.
///
/// If the leaving member was the last admin/owner, MEK rotation is triggered
/// for the remaining members.
#[tauri::command]
pub async fn leave_community(
    community_id: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    // Check our role before leaving — if we're owner, trigger MEK rotation for others
    let my_role = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .and_then(|c| c.my_role.clone())
    };

    // Trigger MEK rotation if an admin/owner is leaving
    if matches!(my_role.as_deref(), Some("owner" | "admin")) {
        // Rotate the key so the departing member can't decrypt future messages
        let new_mek = services::community_service::rotate_mek(&community_id, 2);
        tracing::info!(
            community = %community_id,
            generation = new_mek.generation(),
            "MEK rotated on privileged member departure"
        );
        // TODO: Distribute new MEK to remaining members via Signal sessions
    }

    // Remove from local state
    state.communities.write().remove(&community_id);

    // Remove from SQLite (CASCADE on communities handles channels)
    let owner_key = current_owner_key(state.inner())?;
    let pool = pool.inner().clone();
    let community_id_clone = community_id.clone();
    tokio::task::spawn_blocking(move || {
        let conn = pool.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM communities WHERE owner_key = ? AND id = ?",
            rusqlite::params![owner_key, community_id_clone],
        )
        .map_err(|e| e.to_string())?;
        Ok::<_, String>(())
    })
    .await
    .map_err(|e| e.to_string())??;

    tracing::info!(community = %community_id, "left community");
    Ok(())
}

/// Get message history for a community channel.
#[tauri::command]
pub async fn get_channel_messages(
    channel_id: String,
    limit: u32,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<Vec<Message>, String> {
    let our_key = current_owner_key(state.inner()).unwrap_or_default();

    let pool = pool.inner().clone();
    let channel_id_clone = channel_id.clone();
    let ok = our_key.clone();
    let messages = tokio::task::spawn_blocking(move || {
        let conn = pool.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT id, sender_key, body, timestamp FROM messages \
                 WHERE owner_key = ? AND conversation_id = ? AND conversation_type = 'channel' \
                 ORDER BY timestamp ASC LIMIT ?",
            )
            .map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map(rusqlite::params![ok, channel_id_clone, limit], |row| {
                let sender = db::get_str(row, "sender_key");
                let is_own = sender == our_key;
                Ok(Message {
                    id: db::get_i64(row, "id"),
                    sender_id: sender,
                    body: db::get_str(row, "body"),
                    timestamp: db::get_i64(row, "timestamp"),
                    is_own,
                })
            })
            .map_err(|e| e.to_string())?;

        let mut messages = Vec::new();
        for row in rows {
            messages.push(row.map_err(|e| e.to_string())?);
        }
        Ok::<_, String>(messages)
    })
    .await
    .map_err(|e| e.to_string())??;

    Ok(messages)
}

/// Remove a member from a community.
///
/// The caller must be the community owner or an admin to kick members.
/// Admins cannot kick other admins or the owner.
#[tauri::command]
pub async fn remove_community_member(
    community_id: String,
    public_key: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let owner_key = current_owner_key(state.inner())?;

    // Check caller's role
    let my_role = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .and_then(|c| c.my_role.clone())
            .unwrap_or_default()
    };

    if my_role != "owner" && my_role != "admin" {
        return Err("insufficient permissions: must be owner or admin to remove members".to_string());
    }

    let pool = pool.inner().clone();
    let community_id_clone = community_id.clone();
    let public_key_clone = public_key.clone();
    let my_role_clone = my_role.clone();
    tokio::task::spawn_blocking(move || {
        let conn = pool.lock().map_err(|e| e.to_string())?;

        // Check the target member's role -- admins cannot kick other admins or the owner
        let target_role: Option<String> = conn
            .query_row(
                "SELECT role FROM community_members WHERE owner_key = ? AND community_id = ? AND public_key = ?",
                rusqlite::params![owner_key, community_id_clone, public_key_clone],
                |row| Ok(db::get_str(row, "role")),
            )
            .optional()
            .map_err(|e| e.to_string())?;

        let target_role = target_role.unwrap_or_default();

        if target_role == "owner" {
            return Err("cannot remove the community owner".to_string());
        }
        if my_role_clone == "admin" && target_role == "admin" {
            return Err("admins cannot remove other admins".to_string());
        }

        conn.execute(
            "DELETE FROM community_members WHERE owner_key = ? AND community_id = ? AND public_key = ?",
            rusqlite::params![owner_key, community_id_clone, public_key_clone],
        )
        .map_err(|e| e.to_string())?;

        Ok::<_, String>(())
    })
    .await
    .map_err(|e| e.to_string())??;

    tracing::info!(
        community = %community_id,
        member = %public_key,
        "removed community member"
    );
    Ok(())
}

/// Update a member's role in a community.
///
/// Only the owner or admins can change roles. Admins cannot promote
/// others to owner or change another admin's role.
#[tauri::command]
pub async fn update_member_role(
    community_id: String,
    public_key: String,
    role: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    // Validate the role value
    if !matches!(role.as_str(), "owner" | "admin" | "member") {
        return Err(format!("invalid role: {role}"));
    }

    // Check caller's role
    let my_role = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .and_then(|c| c.my_role.clone())
            .unwrap_or_default()
    };

    if my_role != "owner" && my_role != "admin" {
        return Err(
            "insufficient permissions: must be owner or admin to change roles".to_string(),
        );
    }

    // Only the owner can promote to owner or admin
    if my_role == "admin" && (role == "owner" || role == "admin") {
        return Err("only the owner can promote members to admin or owner".to_string());
    }

    let owner_key = current_owner_key(state.inner())?;
    let pool = pool.inner().clone();
    let role_clone = role.clone();
    let community_id_clone = community_id.clone();
    let public_key_clone = public_key.clone();
    tokio::task::spawn_blocking(move || {
        let conn = pool.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE community_members SET role = ? WHERE owner_key = ? AND community_id = ? AND public_key = ?",
            rusqlite::params![role_clone, owner_key, community_id_clone, public_key_clone],
        )
        .map_err(|e| e.to_string())?;
        Ok::<_, String>(())
    })
    .await
    .map_err(|e| e.to_string())??;

    tracing::info!(
        community = %community_id,
        member = %public_key,
        role = %role,
        "updated member role"
    );
    Ok(())
}

/// Get members of a community from the local cache.
///
/// Community membership is tracked locally -- members are discovered
/// via DHT and cached in `SQLite`. The owner is always included as a
/// member when a community is created.
///
/// Live presence status is cross-referenced from the in-memory friends
/// map so that online friends show their real status instead of "offline".
#[tauri::command]
pub async fn get_community_members(
    community_id: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<Vec<MemberDto>, String> {
    // Clone friends map outside spawn_blocking (parking_lot guard is !Send)
    let friends_snapshot: std::collections::HashMap<String, crate::state::UserStatus> = {
        let friends = state.friends.read();
        friends
            .iter()
            .map(|(k, v)| (k.clone(), v.status))
            .collect()
    };

    let owner_key = current_owner_key(state.inner())?;
    let pool = pool.inner().clone();
    let community_id_clone = community_id.clone();
    let members = tokio::task::spawn_blocking(move || {
        let conn = pool.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT public_key, display_name, role FROM community_members \
                 WHERE owner_key = ? AND community_id = ? ORDER BY role, display_name",
            )
            .map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map(rusqlite::params![owner_key, community_id_clone], |row| {
                let public_key = db::get_str(row, "public_key");

                // Look up live presence from friends state; default to offline
                let status = friends_snapshot
                    .get(&public_key)
                    .copied()
                    .unwrap_or(crate::state::UserStatus::Offline);
                let status_str = match status {
                    crate::state::UserStatus::Online => "online",
                    crate::state::UserStatus::Away => "away",
                    crate::state::UserStatus::Busy => "busy",
                    crate::state::UserStatus::Offline => "offline",
                };

                Ok(MemberDto {
                    public_key,
                    display_name: db::get_str(row, "display_name"),
                    role: db::get_str(row, "role"),
                    status: status_str.to_string(),
                })
            })
            .map_err(|e| e.to_string())?;

        let mut members = Vec::new();
        for row in rows {
            members.push(row.map_err(|e| e.to_string())?);
        }
        Ok::<_, String>(members)
    })
    .await
    .map_err(|e| e.to_string())??;

    Ok(members)
}
