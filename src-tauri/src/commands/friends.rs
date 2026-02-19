use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};

use crate::channels::ChatEvent;
use crate::commands::auth::current_owner_key;
use crate::db::{self, DbPool};
use crate::services;
use crate::state::{FriendState, FriendshipState, SharedState, UserStatus};

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
    pub friendship_state: FriendshipState,
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
            "INSERT OR IGNORE INTO friends (owner_key, public_key, display_name, added_at, friendship_state) VALUES (?1, ?2, ?3, ?4, 'pending_out')",
            rusqlite::params![ok, pk, dn, timestamp],
        )
        .map_err(|e| format!("insert friend: {e}"))?;
        Ok::<(), String>(())
    })
    .await
    .map_err(|e| e.to_string())??;

    // Add to in-memory state as pending (not yet accepted by peer)
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
        mailbox_dht_key: None,
        last_heartbeat_at: None,
        friendship_state: FriendshipState::PendingOut,
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
            friendship_state: "pendingOut".to_string(),
        },
    );

    Ok(())
}

/// Remove a friend.
#[tauri::command]
pub async fn remove_friend(
    public_key: String,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let owner_key = current_owner_key(state.inner())?;

    // Notify the peer BEFORE local removal so we still have their route info
    if let Err(e) = services::message_service::send_to_peer_raw(
        state.inner(),
        pool.inner(),
        &public_key,
        &rekindle_protocol::messaging::envelope::MessagePayload::Unfriended,
    )
    .await
    {
        tracing::warn!(to = %public_key, error = %e, "failed to send unfriend notification (continuing with local removal)");
    }

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

    // Emit event so ALL windows update (not just the one that called the command)
    let _ = app.emit("chat-event", &ChatEvent::FriendRemoved {
        public_key: public_key.clone(),
    });

    tracing::info!(public_key = %public_key, "friend removed");
    Ok(())
}

/// Accept a pending friend request.
#[tauri::command]
#[allow(clippy::too_many_lines)]
pub async fn accept_request(
    public_key: String,
    display_name: String,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let owner_key = current_owner_key(state.inner())?;
    let timestamp = db::timestamp_now();

    // Read stored profile_dht_key, mailbox_dht_key, route_blob, and prekey_bundle BEFORE deleting the pending request
    let (pending_profile_key, pending_mailbox_key, pending_route_blob, pending_prekey_bundle) =
        read_pending_request_data(pool.inner(), &owner_key, &public_key).await?;

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

    // Cache the requester's route blob so send_friend_accept() can deliver immediately
    if let Some(ref blob) = pending_route_blob {
        if !blob.is_empty() {
            let api = state.node.read().as_ref().map(|nh| nh.api.clone());
            if let Some(api) = api {
                let mut dht_mgr = state.dht_manager.write();
                if let Some(mgr) = dht_mgr.as_mut() {
                    mgr.manager.cache_route(&api, &public_key, blob.clone());
                }
            }
        }
    }

    // Add to in-memory state with profile and mailbox keys from the request
    let friend = FriendState {
        public_key: public_key.clone(),
        display_name: display_name.clone(),
        nickname: None,
        status: UserStatus::Offline,
        status_message: None,
        game_info: None,
        group: None,
        unread_count: 0,
        dht_record_key: pending_profile_key.clone(),
        last_seen_at: None,
        local_conversation_key: None,
        remote_conversation_key: None,
        mailbox_dht_key: pending_mailbox_key.clone(),
        last_heartbeat_at: None,
        friendship_state: FriendshipState::Accepted,
    };
    state.friends.write().insert(public_key.clone(), friend);

    // Persist profile_dht_key and mailbox_dht_key to friends table
    if pending_profile_key.is_some() || pending_mailbox_key.is_some() {
        let pool_clone3 = pool.inner().clone();
        let pk3 = public_key.clone();
        let ok3 = current_owner_key(state.inner())?;
        let pdk = pending_profile_key.clone();
        let mdk = pending_mailbox_key;
        tokio::task::spawn_blocking(move || {
            let conn = pool_clone3.lock().map_err(|e| e.to_string())?;
            conn.execute(
                "UPDATE friends SET dht_record_key = COALESCE(?1, dht_record_key), \
                 mailbox_dht_key = COALESCE(?2, mailbox_dht_key) \
                 WHERE owner_key = ?3 AND public_key = ?4",
                rusqlite::params![pdk, mdk, ok3, pk3],
            )
            .map_err(|e| format!("update friend keys: {e}"))?;
            Ok::<(), String>(())
        })
        .await
        .map_err(|e| e.to_string())??;
    }

    // Establish initiator-side Signal session using the requester's stored prekey bundle.
    // We (the acceptor) are the Signal initiator; the requester will be the responder.
    let session_init = if let Some(ref prekey_bytes) = pending_prekey_bundle {
        let signal = state.signal_manager.lock();
        if let Some(handle) = signal.as_ref() {
            if let Ok(bundle) = serde_json::from_slice::<rekindle_crypto::signal::PreKeyBundle>(prekey_bytes) {
                match handle.manager.establish_session(&public_key, &bundle) {
                    Ok(info) => {
                        tracing::info!(peer = %public_key, "established initiator Signal session on accept");
                        Some(info)
                    }
                    Err(e) => {
                        tracing::warn!(peer = %public_key, error = %e, "failed to establish Signal session on accept");
                        None
                    }
                }
            } else {
                tracing::warn!(peer = %public_key, "failed to deserialize stored prekey bundle");
                None
            }
        } else {
            None
        }
    } else {
        tracing::debug!(peer = %public_key, "no stored prekey bundle for session establishment");
        None
    };

    // Send acceptance back via Veilid (includes ephemeral key for responder session)
    services::message_service::send_friend_accept(
        state.inner(),
        pool.inner(),
        &public_key,
        session_init,
    )
    .await
    .unwrap_or_else(|e| {
        tracing::warn!(error = %e, "failed to send friend accept via Veilid");
    });

    // Start watching the friend's profile DHT record for presence
    if let Some(ref dht_key) = pending_profile_key {
        if let Err(e) =
            services::presence_service::watch_friend(state.inner(), &public_key, dht_key).await
        {
            tracing::trace!(error = %e, "failed to watch friend DHT after accepting request");
        }
    }

    let _ = app.emit(
        "chat-event",
        &ChatEvent::FriendRequestAccepted {
            from: public_key,
            display_name,
        },
    );

    Ok(())
}

/// Pending friend request data: `(profile_dht_key, mailbox_dht_key, route_blob, prekey_bundle)`.
type PendingRequestData = (Option<String>, Option<String>, Option<Vec<u8>>, Option<Vec<u8>>);

/// Read `profile_dht_key`, `mailbox_dht_key`, `route_blob`, and `prekey_bundle` from a pending friend request.
async fn read_pending_request_data(
    pool: &DbPool,
    owner_key: &str,
    public_key: &str,
) -> Result<PendingRequestData, String> {
    let pool_clone = pool.clone();
    let ok = owner_key.to_string();
    let pk = public_key.to_string();
    tokio::task::spawn_blocking(move || {
        let conn = pool_clone.lock().map_err(|e| e.to_string())?;
        let row: Option<PendingRequestData> = conn
            .query_row(
                "SELECT profile_dht_key, mailbox_dht_key, route_blob, prekey_bundle FROM pending_friend_requests WHERE owner_key = ?1 AND public_key = ?2",
                rusqlite::params![ok, pk],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        Ok(row.unwrap_or((None, None, None, None)))
    })
    .await
    .map_err(|e| e.to_string())?
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
            friendship_state: f.friendship_state,
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

/// Cancel an outbound pending friend request.
///
/// Only works for friends in `PendingOut` state — removes them from the
/// friends table and in-memory state.
#[tauri::command]
pub async fn cancel_request(
    public_key: String,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let owner_key = current_owner_key(state.inner())?;

    // Only cancel pending_out friends
    let is_pending = state
        .friends
        .read()
        .get(&public_key)
        .is_some_and(|f| f.friendship_state == FriendshipState::PendingOut);
    if !is_pending {
        return Err("Not a pending outbound request".to_string());
    }

    // Delete from DB
    let pool_clone = pool.inner().clone();
    let pk = public_key.clone();
    let ok = owner_key;
    tokio::task::spawn_blocking(move || {
        let conn = pool_clone.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM friends WHERE owner_key = ?1 AND public_key = ?2",
            rusqlite::params![ok, pk],
        )
        .map_err(|e| format!("delete pending friend: {e}"))?;
        Ok::<(), String>(())
    })
    .await
    .map_err(|e| e.to_string())??;

    // Remove from in-memory state
    state.friends.write().remove(&public_key);

    // Emit FriendRemoved so frontend updates
    let _ = app.emit(
        "chat-event",
        &ChatEvent::FriendRemoved {
            public_key: public_key.clone(),
        },
    );

    tracing::info!(public_key = %public_key, "pending friend request cancelled");
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

/// Generate an invite link containing everything needed for a peer to add us.
#[tauri::command]
pub async fn generate_invite(
    state: State<'_, SharedState>,
) -> Result<String, String> {
    // Gather identity info
    let (public_key, display_name, secret_key) = {
        let identity = state.identity.read();
        let id = identity.as_ref().ok_or("identity not set")?;
        let pk = id.public_key.clone();
        let dn = id.display_name.clone();
        let sk = *state.identity_secret.lock();
        let sk = sk.ok_or("signing key not initialized")?;
        (pk, dn, sk)
    };

    let (mailbox_dht_key, profile_dht_key, route_blob) = {
        let node = state.node.read();
        let nh = node.as_ref().ok_or("node not initialized")?;
        let mdk = nh.mailbox_dht_key.clone().ok_or("mailbox DHT key not set")?;
        let pdk = nh.profile_dht_key.clone().ok_or("profile DHT key not set")?;
        let rb = nh.route_blob.clone().ok_or("route not allocated yet — try again in a moment")?;
        (mdk, pdk, rb)
    };

    // Generate a PreKeyBundle
    let prekey_bundle = {
        let signal = state.signal_manager.lock();
        let handle = signal.as_ref().ok_or("signal manager not initialized")?;
        let bundle = handle
            .manager
            .generate_prekey_bundle(1, Some(1))
            .map_err(|e| format!("generate prekey bundle: {e}"))?;
        serde_json::to_vec(&bundle).map_err(|e| format!("serialize prekey bundle: {e}"))?
    };

    let blob = rekindle_protocol::messaging::create_invite_blob(
        &secret_key,
        &public_key,
        &display_name,
        &mailbox_dht_key,
        &profile_dht_key,
        &route_blob,
        &prekey_bundle,
    );
    Ok(rekindle_protocol::messaging::encode_invite_url(&blob))
}

/// Add a friend from a `rekindle://` invite string.
#[tauri::command]
pub async fn add_friend_from_invite(
    invite_string: String,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    // Decode and verify the invite
    let blob = rekindle_protocol::messaging::decode_invite_url(&invite_string)?;
    rekindle_protocol::messaging::verify_invite_blob(&blob)?;

    let owner_key = current_owner_key(state.inner())?;

    // Prevent adding yourself
    if blob.public_key == owner_key {
        return Err("You cannot add yourself as a friend".to_string());
    }

    let timestamp = db::timestamp_now();

    // Insert into SQLite with profile and mailbox keys from the invite
    let pool_clone = pool.inner().clone();
    let pk = blob.public_key.clone();
    let dn = blob.display_name.clone();
    let ok = owner_key;
    let profile_key = blob.profile_dht_key.clone();
    let mailbox_key = blob.mailbox_dht_key.clone();
    tokio::task::spawn_blocking(move || {
        let conn = pool_clone.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT OR IGNORE INTO friends (owner_key, public_key, display_name, added_at, dht_record_key, mailbox_dht_key, friendship_state) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending_out')",
            rusqlite::params![ok, pk, dn, timestamp, profile_key, mailbox_key],
        )
        .map_err(|e| format!("insert friend: {e}"))?;
        Ok::<(), String>(())
    })
    .await
    .map_err(|e| e.to_string())??;

    // Add to in-memory state (pending until peer accepts)
    let friend = FriendState {
        public_key: blob.public_key.clone(),
        display_name: blob.display_name.clone(),
        nickname: None,
        status: UserStatus::Offline,
        status_message: None,
        game_info: None,
        group: None,
        unread_count: 0,
        dht_record_key: Some(blob.profile_dht_key.clone()),
        last_seen_at: None,
        local_conversation_key: None,
        remote_conversation_key: None,
        mailbox_dht_key: Some(blob.mailbox_dht_key.clone()),
        last_heartbeat_at: None,
        friendship_state: FriendshipState::PendingOut,
    };
    state.friends.write().insert(blob.public_key.clone(), friend);

    // Cache route blob, establish Signal session, and try mailbox for fresh route
    setup_invite_contact(state.inner(), &blob).await;

    // Send friend request via Veilid (includes our profile key + route blob + mailbox key)
    services::message_service::send_friend_request(
        state.inner(),
        pool.inner(),
        &blob.public_key,
        "Added via invite link",
    )
    .await
    .unwrap_or_else(|e| {
        tracing::warn!(error = %e, "failed to send friend request via Veilid");
    });

    // Start watching the friend's profile DHT record for presence
    if let Err(e) = services::presence_service::watch_friend(
        state.inner(),
        &blob.public_key,
        &blob.profile_dht_key,
    )
    .await
    {
        tracing::trace!(error = %e, "failed to watch friend DHT after invite add");
    }

    // Emit event so frontend updates
    let _ = app.emit(
        "chat-event",
        &ChatEvent::FriendAdded {
            public_key: blob.public_key.clone(),
            display_name: blob.display_name.clone(),
            friendship_state: "pendingOut".to_string(),
        },
    );

    tracing::info!(public_key = %blob.public_key, "friend added from invite");
    Ok(())
}

/// Cache invite route blob, establish Signal session, and refresh route from mailbox.
async fn setup_invite_contact(
    state: &std::sync::Arc<crate::state::AppState>,
    blob: &rekindle_protocol::messaging::envelope::InviteBlob,
) {
    // Cache the route blob from the invite for immediate contact
    let api = state.node.read().as_ref().map(|nh| nh.api.clone());
    if let Some(ref api) = api {
        let mut dht_mgr = state.dht_manager.write();
        if let Some(mgr) = dht_mgr.as_mut() {
            mgr.manager.cache_route(api, &blob.public_key, blob.route_blob.clone());
        }
    }

    // Establish Signal session from invite's PreKeyBundle
    if let Ok(bundle) =
        serde_json::from_slice::<rekindle_crypto::signal::PreKeyBundle>(&blob.prekey_bundle)
    {
        let signal = state.signal_manager.lock();
        if let Some(handle) = signal.as_ref() {
            match handle.manager.establish_session(&blob.public_key, &bundle) {
                Ok(_init_info) => {
                    tracing::info!(peer = %blob.public_key, "established Signal session from invite");
                }
                Err(e) => {
                    tracing::warn!(error = %e, "failed to establish Signal session from invite");
                }
            }
        }
    }

    // Try reading the peer's mailbox for a fresh route blob (invite may be stale)
    let rc = state.node.read().as_ref().map(|nh| nh.routing_context.clone());
    if let Some(rc) = rc {
        match rekindle_protocol::dht::mailbox::read_peer_mailbox_route(&rc, &blob.mailbox_dht_key).await {
            Ok(Some(fresh_blob)) if !fresh_blob.is_empty() => {
                if let Some(ref api) = api {
                    let mut dht_mgr = state.dht_manager.write();
                    if let Some(mgr) = dht_mgr.as_mut() {
                        mgr.manager.cache_route(api, &blob.public_key, fresh_blob);
                    }
                }
                tracing::debug!("refreshed route blob from peer's mailbox");
            }
            _ => tracing::trace!("no fresh route blob in peer mailbox — using invite blob"),
        }
    }
}

/// Block a friend — removes them and rotates our profile DHT key so they
/// can no longer watch our presence.
#[tauri::command]
pub async fn block_friend(
    public_key: String,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let owner_key = current_owner_key(state.inner())?;
    let timestamp = db::timestamp_now();

    // Remove from friends + add to blocked list in one transaction
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
        conn.execute(
            "INSERT OR IGNORE INTO blocked_users (owner_key, public_key, blocked_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![ok, pk, timestamp],
        )
        .map_err(|e| format!("insert blocked user: {e}"))?;
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

    // Rotate our profile DHT key so the blocked user can't watch us anymore
    rotate_profile_key(state.inner(), pool.inner()).await?;

    let _ = app.emit(
        "chat-event",
        &ChatEvent::FriendRemoved {
            public_key: public_key.clone(),
        },
    );

    tracing::info!(public_key = %public_key, "friend blocked and profile key rotated");
    Ok(())
}

/// Rotate the profile DHT key: create a new profile record, copy data,
/// update state/DB, and notify all remaining friends via `ProfileKeyRotated`.
async fn rotate_profile_key(
    state: &std::sync::Arc<crate::state::AppState>,
    pool: &DbPool,
) -> Result<(), String> {
    // Create a new profile DHT record with a fresh keypair.
    // Clone the routing_context out before .await (parking_lot guards are !Send).
    let routing_context = {
        let node = state.node.read();
        let nh = node.as_ref().ok_or("node not initialized")?;
        nh.routing_context.clone()
    };
    let temp_mgr = rekindle_protocol::dht::DHTManager::new(routing_context.clone());
    let (new_key, new_keypair) = temp_mgr
        .create_record(8)
        .await
        .map_err(|e| format!("create new profile record: {e}"))?;

    // Copy current profile data to the new record
    let (old_key_str, display_name, status_bytes, route_blob) = {
        let node = state.node.read();
        let nh = node.as_ref().ok_or("node not initialized")?;
        let ok = nh.profile_dht_key.clone().unwrap_or_default();
        let identity = state.identity.read();
        let id = identity.as_ref().ok_or("identity not set")?;
        let dn = id.display_name.clone();
        let status = id.status as u8;
        let rb = nh.route_blob.clone().unwrap_or_default();
        (ok, dn, vec![status], rb)
    };

    // Read prekey from signal manager
    let prekey_bytes = {
        let signal = state.signal_manager.lock();
        if let Some(handle) = signal.as_ref() {
            match handle.manager.generate_prekey_bundle(1, Some(1)) {
                Ok(bundle) => serde_json::to_vec(&bundle).unwrap_or_default(),
                Err(_) => Vec::new(),
            }
        } else {
            Vec::new()
        }
    };

    let record_key: veilid_core::RecordKey = new_key
        .parse()
        .map_err(|e| format!("invalid new profile key: {e}"))?;

    // Write profile subkeys to new record
    // Subkey 0: display name, 1: status, 5: prekey, 6: route blob
    let _ = routing_context
        .set_dht_value(record_key.clone(), 0, display_name.into_bytes(), None)
        .await;
    let _ = routing_context
        .set_dht_value(record_key.clone(), 1, status_bytes, None)
        .await;
    let _ = routing_context
        .set_dht_value(record_key.clone(), 5, prekey_bytes, None)
        .await;
    let _ = routing_context
        .set_dht_value(record_key.clone(), 6, route_blob, None)
        .await;

    // Update NodeHandle
    {
        let mut node = state.node.write();
        if let Some(nh) = node.as_mut() {
            nh.profile_dht_key = Some(new_key.clone());
            nh.profile_owner_keypair.clone_from(&new_keypair);
        }
    }

    // Update SQLite (both dht_record_key and dht_owner_keypair)
    let pool_clone = pool.clone();
    let nk = new_key.clone();
    let keypair_str = new_keypair.as_ref().map(std::string::ToString::to_string);
    let owner_key = state
        .identity
        .read()
        .as_ref()
        .map(|id| id.public_key.clone())
        .unwrap_or_default();
    tokio::task::spawn_blocking(move || {
        let conn = pool_clone.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE identity SET dht_record_key = ?1, dht_owner_keypair = COALESCE(?3, dht_owner_keypair) WHERE public_key = ?2",
            rusqlite::params![nk, owner_key, keypair_str],
        )
        .map_err(|e| format!("update identity dht key: {e}"))?;
        Ok::<(), String>(())
    })
    .await
    .map_err(|e| e.to_string())??;

    // Notify all remaining friends about the new profile key
    let friend_keys: Vec<String> = {
        let friends = state.friends.read();
        friends.keys().cloned().collect()
    };
    let payload = rekindle_protocol::messaging::envelope::MessagePayload::ProfileKeyRotated {
        new_profile_dht_key: new_key.clone(),
    };
    for fk in &friend_keys {
        if let Err(e) =
            services::message_service::send_to_peer_raw(state, pool, fk, &payload).await
        {
            tracing::warn!(to = %fk, error = %e, "failed to send ProfileKeyRotated");
        }
    }

    tracing::info!(
        old_key = %old_key_str,
        new_key = %new_key,
        "profile DHT key rotated — {} friends notified",
        friend_keys.len()
    );
    Ok(())
}

/// Re-emit presence events for all non-offline friends.
///
/// Called by the frontend after hydration completes so that event listeners
/// (registered before hydration) receive the current friend presence state.
/// Waits for Veilid network readiness (up to 15s) before syncing from DHT
/// so that `state.friends` has fresh data rather than stale Offline defaults.
#[tauri::command]
pub async fn emit_friends_presence(
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    // Wait for network readiness before attempting DHT sync (up to 15s).
    // Without this, sync_friends_now() silently no-ops when not attached.
    let mut rx = state.network_ready_rx.clone();
    let _ready = tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            if *rx.borrow_and_update() {
                return true;
            }
            if rx.changed().await.is_err() {
                return false;
            }
        }
    })
    .await
    .unwrap_or(false);

    // Best-effort sync from DHT so we have fresh status data
    let _ = services::sync_service::sync_friends_now(&state, &app).await;

    let friends: Vec<(String, UserStatus)> = {
        let friends = state.friends.read();
        friends.values().map(|f| (f.public_key.clone(), f.status)).collect()
    };
    for (key, status) in friends {
        if status != UserStatus::Offline {
            // Emit FriendOnline first so the frontend transitions the friend
            // out of the offline visual group before updating the specific status.
            let _ = app.emit("presence-event",
                &crate::channels::PresenceEvent::FriendOnline {
                    public_key: key.clone(),
                });
            let _ = app.emit("presence-event",
                &crate::channels::PresenceEvent::StatusChanged {
                    public_key: key,
                    status: format!("{status:?}").to_lowercase(),
                    status_message: None,
                });
        }
    }
    Ok(())
}
