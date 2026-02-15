use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};

use crate::channels::ChatEvent;
use crate::commands::auth::current_owner_key;
use crate::db::{self, DbPool};
use crate::services;
use crate::state::{FriendState, SharedState, UserStatus};

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FriendResponse {
    pub public_key: String,
    pub display_name: String,
    pub nickname: Option<String>,
    pub status: UserStatus,
    pub status_message: Option<String>,
    pub game_info: Option<crate::state::GameInfoState>,
    pub group: Option<String>,
    pub unread_count: u32,
    pub last_seen_at: Option<i64>,
}

/// A pending friend request stored in `SQLite`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingFriendRequest {
    pub public_key: String,
    pub display_name: String,
    pub message: String,
    pub received_at: i64,
}

/// Get all persisted pending friend requests for the current identity.
#[tauri::command]
pub async fn get_pending_requests(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<Vec<PendingFriendRequest>, String> {
    let owner_key = current_owner_key(state.inner())?;
    let pool_clone = pool.inner().clone();
    tokio::task::spawn_blocking(move || {
        let conn = pool_clone.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT public_key, display_name, message, received_at \
                 FROM pending_friend_requests WHERE owner_key = ?1 ORDER BY received_at",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(rusqlite::params![owner_key], |row| {
                Ok(PendingFriendRequest {
                    public_key: row.get(0)?,
                    display_name: row.get(1)?,
                    message: row.get(2)?,
                    received_at: row.get(3)?,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| e.to_string())?);
        }
        Ok(results)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Add a friend by their public key.
#[tauri::command]
pub async fn add_friend(
    public_key: String,
    display_name: String,
    message: String,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    // Validate public key format (64 hex chars = 32 bytes)
    if public_key.len() != 64 || hex::decode(&public_key).is_err() {
        return Err("Invalid public key — must be a 64-character hex string".to_string());
    }

    let owner_key = current_owner_key(state.inner())?;

    // Prevent adding yourself
    if public_key == owner_key {
        return Err("You cannot add yourself as a friend".to_string());
    }

    let timestamp = db::timestamp_now();

    // Insert into SQLite
    let pool_clone = pool.inner().clone();
    let pk = public_key.clone();
    let dn = display_name.clone();
    let ok = owner_key.clone();
    tokio::task::spawn_blocking(move || {
        let conn = pool_clone.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT OR IGNORE INTO friends (owner_key, public_key, display_name, added_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![ok, pk, dn, timestamp],
        )
        .map_err(|e| format!("insert friend: {e}"))?;
        Ok::<(), String>(())
    })
    .await
    .map_err(|e| e.to_string())??;

    // Add to in-memory state
    let friend = FriendState {
        public_key: public_key.clone(),
        display_name: display_name.clone(),
        nickname: None,
        status: UserStatus::Offline,
        status_message: None,
        game_info: None,
        group: None,
        unread_count: 0,
        dht_record_key: None,
        last_seen_at: None,
        local_conversation_key: None,
        remote_conversation_key: None,
    };
    state.friends.write().insert(public_key.clone(), friend);

    // Send friend request via Veilid
    services::message_service::send_friend_request(
        state.inner(),
        pool.inner(),
        &public_key,
        &message,
    )
    .await
    .unwrap_or_else(|e| {
        tracing::warn!(error = %e, "failed to send friend request via Veilid (peer may be offline)");
    });

    // Emit event so frontend updates
    let _ = app.emit(
        "chat-event",
        &ChatEvent::FriendAdded {
            public_key,
            display_name,
        },
    );

    Ok(())
}

/// Remove a friend.
#[tauri::command]
pub async fn remove_friend(
    public_key: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let owner_key = current_owner_key(state.inner())?;

    // Remove from SQLite
    let pool_clone = pool.inner().clone();
    let pk = public_key.clone();
    let ok = owner_key;
    tokio::task::spawn_blocking(move || {
        let conn = pool_clone.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM friends WHERE owner_key = ?1 AND public_key = ?2",
            rusqlite::params![ok, pk],
        )
        .map_err(|e| format!("delete friend: {e}"))?;
        Ok::<(), String>(())
    })
    .await
    .map_err(|e| e.to_string())??;

    // Remove from in-memory state and unregister DHT key mapping
    let dht_key = {
        let mut friends = state.friends.write();
        let removed = friends.remove(&public_key);
        removed.and_then(|f| f.dht_record_key)
    };
    if let Some(ref dht_key) = dht_key {
        let mut dht_mgr = state.dht_manager.write();
        if let Some(mgr) = dht_mgr.as_mut() {
            mgr.unregister_friend_dht_key(dht_key);
        }
    }

    // Update DHT friend list record
    if let Err(e) = services::message_service::push_friend_list_update(state.inner()).await {
        tracing::warn!(error = %e, "failed to update DHT friend list after removal");
    }

    tracing::info!(public_key = %public_key, "friend removed");
    Ok(())
}

/// Accept a pending friend request.
#[tauri::command]
pub async fn accept_request(
    public_key: String,
    display_name: String,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let owner_key = current_owner_key(state.inner())?;
    let timestamp = db::timestamp_now();

    // Insert into friends and delete from pending_friend_requests atomically
    let pool_clone = pool.inner().clone();
    let pk = public_key.clone();
    let dn = display_name.clone();
    let ok = owner_key;
    tokio::task::spawn_blocking(move || {
        let conn = pool_clone.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT OR IGNORE INTO friends (owner_key, public_key, display_name, added_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![ok, pk, dn, timestamp],
        )
        .map_err(|e| format!("insert friend: {e}"))?;
        conn.execute(
            "DELETE FROM pending_friend_requests WHERE owner_key = ?1 AND public_key = ?2",
            rusqlite::params![ok, pk],
        )
        .map_err(|e| format!("delete pending request: {e}"))?;
        Ok::<(), String>(())
    })
    .await
    .map_err(|e| e.to_string())??;

    // Add to in-memory state
    let friend = FriendState {
        public_key: public_key.clone(),
        display_name: display_name.clone(),
        nickname: None,
        status: UserStatus::Offline,
        status_message: None,
        game_info: None,
        group: None,
        unread_count: 0,
        dht_record_key: None,
        last_seen_at: None,
        local_conversation_key: None,
        remote_conversation_key: None,
    };
    state.friends.write().insert(public_key.clone(), friend);

    // Send acceptance back via Veilid
    services::message_service::send_friend_accept(
        state.inner(),
        pool.inner(),
        &public_key,
    )
    .await
    .unwrap_or_else(|e| {
        tracing::warn!(error = %e, "failed to send friend accept via Veilid");
    });

    let _ = app.emit(
        "chat-event",
        &ChatEvent::FriendRequestAccepted {
            from: public_key,
            display_name,
        },
    );

    Ok(())
}

/// Get the full friends list.
#[tauri::command]
pub async fn get_friends(state: State<'_, SharedState>) -> Result<Vec<FriendResponse>, String> {
    let friends = state.friends.read();
    let list: Vec<FriendResponse> = friends
        .values()
        .map(|f| FriendResponse {
            public_key: f.public_key.clone(),
            display_name: f.display_name.clone(),
            nickname: f.nickname.clone(),
            status: f.status,
            status_message: f.status_message.clone(),
            game_info: f.game_info.clone(),
            group: f.group.clone(),
            unread_count: f.unread_count,
            last_seen_at: f.last_seen_at,
        })
        .collect();
    Ok(list)
}

/// Reject a pending friend request.
/// Sends a rejection message to the peer and removes the request from the database.
#[tauri::command]
pub async fn reject_request(
    public_key: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let owner_key = current_owner_key(state.inner())?;

    // Delete from pending_friend_requests
    let pool_clone = pool.inner().clone();
    let pk = public_key.clone();
    let ok = owner_key;
    tokio::task::spawn_blocking(move || {
        let conn = pool_clone.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM pending_friend_requests WHERE owner_key = ?1 AND public_key = ?2",
            rusqlite::params![ok, pk],
        )
        .map_err(|e| format!("delete pending request: {e}"))?;
        Ok::<(), String>(())
    })
    .await
    .map_err(|e| e.to_string())??;

    // Send rejection to the peer via Veilid
    services::message_service::send_friend_reject(
        state.inner(),
        pool.inner(),
        &public_key,
    )
    .await
    .unwrap_or_else(|e| {
        tracing::warn!(error = %e, "failed to send friend reject via Veilid (peer may be offline)");
    });

    tracing::info!(public_key = %public_key, "friend request rejected");
    Ok(())
}

/// Create a new friend group.
#[tauri::command]
pub async fn create_friend_group(
    name: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<i64, String> {
    let owner_key = current_owner_key(state.inner())?;
    let pool_clone = pool.inner().clone();
    tokio::task::spawn_blocking(move || {
        let conn = pool_clone.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO friend_groups (owner_key, name) VALUES (?1, ?2)",
            rusqlite::params![owner_key, name],
        )
        .map_err(|e| format!("create friend group: {e}"))?;
        Ok::<i64, String>(conn.last_insert_rowid())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Rename a friend group.
#[tauri::command]
pub async fn rename_friend_group(
    group_id: i64,
    name: String,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let pool_clone = pool.inner().clone();
    tokio::task::spawn_blocking(move || {
        let conn = pool_clone.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE friend_groups SET name = ?1 WHERE id = ?2",
            rusqlite::params![name, group_id],
        )
        .map_err(|e| format!("rename friend group: {e}"))?;
        Ok::<(), String>(())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Move a friend into a group (or remove from group with `group_id` = null).
#[tauri::command]
pub async fn move_friend_to_group(
    public_key: String,
    group_id: Option<i64>,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let owner_key = current_owner_key(state.inner())?;
    let pool_clone = pool.inner().clone();
    let pk = public_key.clone();
    let ok = owner_key;
    tokio::task::spawn_blocking(move || {
        let conn = pool_clone.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE friends SET group_id = ?1 WHERE owner_key = ?2 AND public_key = ?3",
            rusqlite::params![group_id, ok, pk],
        )
        .map_err(|e| format!("move friend to group: {e}"))?;
        Ok::<(), String>(())
    })
    .await
    .map_err(|e| e.to_string())??;

    // Update in-memory — resolve group name from DB
    if let Some(group_id) = group_id {
        let pool_clone2 = pool.inner().clone();
        let group_name: Option<String> = tokio::task::spawn_blocking(move || {
            let conn = pool_clone2.lock().map_err(|e| e.to_string())?;
            conn.query_row(
                "SELECT name FROM friend_groups WHERE id = ?1",
                rusqlite::params![group_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| format!("query group name: {e}"))
        })
        .await
        .map_err(|e| e.to_string())??;

        let mut friends = state.friends.write();
        if let Some(friend) = friends.get_mut(&public_key) {
            friend.group = group_name;
        }
    } else {
        let mut friends = state.friends.write();
        if let Some(friend) = friends.get_mut(&public_key) {
            friend.group = None;
        }
    }

    Ok(())
}
