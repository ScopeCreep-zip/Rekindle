use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};

use crate::channels::ChatEvent;
use crate::db::{self, DbPool};
use crate::db_helpers::{db_call, db_call_or_default};
use crate::services;
use crate::state::{FriendState, FriendshipState, SharedState, UserStatus};
use crate::state_helpers;

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
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    db_call(pool.inner(), move |conn| {
        let mut stmt = conn.prepare(
            "SELECT public_key, display_name, message, received_at \
             FROM pending_friend_requests WHERE owner_key = ?1 ORDER BY received_at",
        )?;
        let rows = stmt.query_map(rusqlite::params![owner_key], |row| {
            Ok(PendingFriendRequest {
                public_key: row.get(0)?,
                display_name: row.get(1)?,
                message: row.get(2)?,
                received_at: row.get(3)?,
            })
        })?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    })
    .await
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

    let owner_key = state_helpers::current_owner_key(state.inner())?;

    // Prevent adding yourself
    if public_key == owner_key {
        return Err("You cannot add yourself as a friend".to_string());
    }

    // Prevent adding a blocked user
    if is_user_blocked(pool.inner(), &owner_key, &public_key).await {
        return Err("Cannot add a blocked user. Unblock them first.".to_string());
    }

    let timestamp = db::timestamp_now();

    // Insert into SQLite
    let pk = public_key.clone();
    let dn = display_name.clone();
    let ok = owner_key.clone();
    db_call(pool.inner(), move |conn| {
        conn.execute(
            "INSERT OR IGNORE INTO friends (owner_key, public_key, display_name, added_at, friendship_state) VALUES (?1, ?2, ?3, ?4, 'pending_out')",
            rusqlite::params![ok, pk, dn, timestamp],
        )?;
        Ok(())
    })
    .await?;

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
        None,
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
    let owner_key = state_helpers::current_owner_key(state.inner())?;

    // ── Local cleanup (fast, never blocks on network) ────────────────

    // Queue Unfriended message for reliable retry delivery (DB insert only).
    // Cleared when peer sends UnfriendedAck, or dropped after max retries (20 × 30s).
    let _ = services::message_service::build_and_queue_envelope(
        state.inner(),
        pool.inner(),
        &public_key,
        &rekindle_protocol::messaging::envelope::MessagePayload::Unfriended,
    )
    .await;

    // Remove from SQLite (permanent — friend won't reappear on restart)
    // Also delete any pending_friend_requests row to prevent stale rows blocking future invites
    let pk = public_key.clone();
    let ok = owner_key;
    db_call(pool.inner(), move |conn| {
        conn.execute(
            "DELETE FROM friends WHERE owner_key = ?1 AND public_key = ?2",
            rusqlite::params![ok, pk],
        )?;
        conn.execute(
            "DELETE FROM pending_friend_requests WHERE owner_key = ?1 AND public_key = ?2",
            rusqlite::params![ok, pk],
        )?;
        Ok(())
    })
    .await?;

    // Mark as Removing in-memory (instead of deleting immediately) so that
    // sync_service can still look up the friend's routing info (mailbox key,
    // DHT record key) when retrying the queued Unfriended notification.
    // The entry is hidden from get_friends and cleaned up after a grace period.
    {
        let mut friends = state.friends.write();
        if let Some(friend) = friends.get_mut(&public_key) {
            friend.friendship_state = FriendshipState::Removing;
        }
    }

    // Emit event so ALL windows update immediately
    let _ = app.emit(
        "chat-event",
        &ChatEvent::FriendRemoved {
            public_key: public_key.clone(),
        },
    );

    tracing::info!(public_key = %public_key, "friend removed");

    // ── Background network operations (can be slow — DHT reads/writes) ──

    let state_clone = state.inner().clone();
    let pool_clone = pool.inner().clone();
    let pk_clone = public_key.clone();
    tokio::spawn(async move {
        // Best-effort immediate send — peer may be offline or route stale
        if let Err(e) = services::message_service::send_to_peer_raw(
            &state_clone,
            &pool_clone,
            &pk_clone,
            &rekindle_protocol::messaging::envelope::MessagePayload::Unfriended,
        )
        .await
        {
            tracing::warn!(to = %pk_clone, error = %e, "failed to send unfriend notification");
        }

        // Unregister DHT presence key mapping (stops watching their status)
        // and invalidate route cache so stale routes aren't reused on re-add
        let dht_key = state_helpers::friend_dht_key(&state_clone, &pk_clone);
        {
            let mut dht_mgr = state_clone.dht_manager.write();
            if let Some(mgr) = dht_mgr.as_mut() {
                if let Some(ref dht_key) = dht_key {
                    mgr.unregister_friend_dht_key(dht_key);
                }
                mgr.manager.invalidate_route_for_peer(&pk_clone);
            }
        }

        // Update DHT friend list record (publishes list without the removed friend)
        if let Err(e) = services::message_service::push_friend_list_update(&state_clone).await {
            tracing::warn!(error = %e, "failed to update DHT friend list after removal");
        }

        // Grace period: fully remove from in-memory state after retries are exhausted.
        // 10 minutes matches the max retry window (20 retries × 30s intervals).
        tokio::time::sleep(std::time::Duration::from_secs(600)).await;
        let mut friends = state_clone.friends.write();
        // Only remove if still in Removing state (user may have re-added them)
        if friends
            .get(&pk_clone)
            .is_some_and(|f| matches!(f.friendship_state, FriendshipState::Removing))
        {
            friends.remove(&pk_clone);
            // Clean up Signal session now that retries are done
            let signal = state_clone.signal_manager.lock();
            if let Some(handle) = signal.as_ref() {
                let _ = handle.manager.delete_session(&pk_clone);
            }
            tracing::debug!(public_key = %pk_clone, "cleaned up Removing friend after grace period");
        }
    });

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
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    let timestamp = db::timestamp_now();

    // Read stored profile_dht_key, mailbox_dht_key, route_blob, prekey_bundle, and invite_id BEFORE deleting the pending request
    let (
        pending_profile_key,
        pending_mailbox_key,
        pending_route_blob,
        pending_prekey_bundle,
        pending_invite_id,
    ) = read_pending_request_data(pool.inner(), &owner_key, &public_key).await?;

    // Insert into friends and delete from pending_friend_requests atomically
    let pk = public_key.clone();
    let dn = display_name.clone();
    let ok = owner_key;
    db_call(pool.inner(), move |conn| {
        conn.execute(
            "INSERT OR IGNORE INTO friends (owner_key, public_key, display_name, added_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![ok, pk, dn, timestamp],
        )?;
        conn.execute(
            "DELETE FROM pending_friend_requests WHERE owner_key = ?1 AND public_key = ?2",
            rusqlite::params![ok, pk],
        )?;
        Ok(())
    })
    .await?;

    // Cache the requester's route blob so send_friend_accept() can deliver immediately
    if let Some(ref blob) = pending_route_blob {
        if !blob.is_empty() {
            let api = state_helpers::veilid_api(state.inner());
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
        let pk3 = public_key.clone();
        let ok3 = state_helpers::current_owner_key(state.inner())?;
        let pdk = pending_profile_key.clone();
        let mdk = pending_mailbox_key;
        db_call(pool.inner(), move |conn| {
            conn.execute(
                "UPDATE friends SET dht_record_key = COALESCE(?1, dht_record_key), \
                 mailbox_dht_key = COALESCE(?2, mailbox_dht_key) \
                 WHERE owner_key = ?3 AND public_key = ?4",
                rusqlite::params![pdk, mdk, ok3, pk3],
            )?;
            Ok(())
        })
        .await?;
    }

    // Establish initiator-side Signal session using the requester's stored prekey bundle.
    // We (the acceptor) are the Signal initiator; the requester will be the responder.
    // Clear any stale session first (e.g., from a previous friendship that was removed).
    let session_init = if let Some(ref prekey_bytes) = pending_prekey_bundle {
        let signal = state.signal_manager.lock();
        if let Some(handle) = signal.as_ref() {
            let _ = handle.manager.delete_session(&public_key);
            if let Ok(bundle) =
                serde_json::from_slice::<rekindle_crypto::signal::PreKeyBundle>(prekey_bytes)
            {
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

    // Mark the originating invite as 'accepted' (if this request came from one)
    if let Some(ref iid) = pending_invite_id {
        let ok = state_helpers::current_owner_key(state.inner()).unwrap_or_default();
        crate::invite_helpers::mark_invite_accepted(pool.inner(), &ok, iid);
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

/// Pending friend request data: `(profile_dht_key, mailbox_dht_key, route_blob, prekey_bundle, invite_id)`.
type PendingRequestData = (
    Option<String>,
    Option<String>,
    Option<Vec<u8>>,
    Option<Vec<u8>>,
    Option<String>,
);

/// Read `profile_dht_key`, `mailbox_dht_key`, `route_blob`, `prekey_bundle`, and `invite_id` from a pending friend request.
async fn read_pending_request_data(
    pool: &DbPool,
    owner_key: &str,
    public_key: &str,
) -> Result<PendingRequestData, String> {
    let ok = owner_key.to_string();
    let pk = public_key.to_string();
    db_call(pool, move |conn| {
        let row: Option<PendingRequestData> = conn
            .query_row(
                "SELECT profile_dht_key, mailbox_dht_key, route_blob, prekey_bundle, invite_id FROM pending_friend_requests WHERE owner_key = ?1 AND public_key = ?2",
                rusqlite::params![ok, pk],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .optional()?;
        Ok(row.unwrap_or((None, None, None, None, None)))
    })
    .await
}

/// Get the full friends list.
#[tauri::command]
pub async fn get_friends(state: State<'_, SharedState>) -> Result<Vec<FriendResponse>, String> {
    let friends = state.friends.read();
    let list: Vec<FriendResponse> = friends
        .values()
        .filter(|f| !matches!(f.friendship_state, FriendshipState::Removing))
        .map(|f| {
            let is_accepted = f.friendship_state == FriendshipState::Accepted;
            FriendResponse {
                public_key: f.public_key.clone(),
                display_name: f.display_name.clone(),
                nickname: f.nickname.clone(),
                status: if is_accepted {
                    f.status
                } else {
                    UserStatus::Offline
                },
                status_message: if is_accepted {
                    f.status_message.clone()
                } else {
                    None
                },
                game_info: if is_accepted {
                    f.game_info.clone()
                } else {
                    None
                },
                group: f.group.clone(),
                unread_count: f.unread_count,
                last_seen_at: f.last_seen_at,
                friendship_state: f.friendship_state,
            }
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
    let owner_key = state_helpers::current_owner_key(state.inner())?;

    // Read pending request data BEFORE deleting — we need the route blob for delivery
    // and invite_id for tracking.
    let (_, _, pending_route_blob, _, invite_id) =
        read_pending_request_data(pool.inner(), &owner_key, &public_key).await?;

    // Delete from pending_friend_requests
    let pk = public_key.clone();
    let ok = owner_key.clone();
    db_call(pool.inner(), move |conn| {
        conn.execute(
            "DELETE FROM pending_friend_requests WHERE owner_key = ?1 AND public_key = ?2",
            rusqlite::params![ok, pk],
        )?;
        Ok(())
    })
    .await?;

    // Mark the originating invite as 'rejected' (if this request came from one)
    if let Some(ref iid) = invite_id {
        crate::invite_helpers::mark_invite_rejected(pool.inner(), &owner_key, iid);
    }

    // Cache the requester's route blob so send_friend_reject can deliver immediately.
    // The requester is NOT in state.friends (only in pending_friend_requests), so
    // without this cache, send_envelope_to_peer's route lookup would fail.
    // Mirrors accept_request's route caching at lines 311-322.
    if let Some(ref blob) = pending_route_blob {
        if !blob.is_empty() {
            let api = state_helpers::veilid_api(state.inner());
            if let Some(api) = api {
                let mut dht_mgr = state.dht_manager.write();
                if let Some(mgr) = dht_mgr.as_mut() {
                    mgr.manager.cache_route(&api, &public_key, blob.clone());
                }
            }
        }
    }

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
    let owner_key = state_helpers::current_owner_key(state.inner())?;

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
    let pk = public_key.clone();
    let ok = owner_key;
    db_call(pool.inner(), move |conn| {
        conn.execute(
            "DELETE FROM friends WHERE owner_key = ?1 AND public_key = ?2",
            rusqlite::params![ok, pk],
        )?;
        Ok(())
    })
    .await?;

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
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    db_call(pool.inner(), move |conn| {
        conn.execute(
            "INSERT INTO friend_groups (owner_key, name) VALUES (?1, ?2)",
            rusqlite::params![owner_key, name],
        )?;
        Ok(conn.last_insert_rowid())
    })
    .await
}

/// Rename a friend group.
#[tauri::command]
pub async fn rename_friend_group(
    group_id: i64,
    name: String,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    db_call(pool.inner(), move |conn| {
        conn.execute(
            "UPDATE friend_groups SET name = ?1 WHERE id = ?2",
            rusqlite::params![name, group_id],
        )?;
        Ok(())
    })
    .await
}

/// Move a friend into a group (or remove from group with `group_id` = null).
#[tauri::command]
pub async fn move_friend_to_group(
    public_key: String,
    group_id: Option<i64>,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    let pk = public_key.clone();
    let ok = owner_key;
    db_call(pool.inner(), move |conn| {
        conn.execute(
            "UPDATE friends SET group_id = ?1 WHERE owner_key = ?2 AND public_key = ?3",
            rusqlite::params![group_id, ok, pk],
        )?;
        Ok(())
    })
    .await?;

    // Update in-memory — resolve group name from DB
    if let Some(group_id) = group_id {
        let group_name: Option<String> = db_call(pool.inner(), move |conn| {
            let name = conn
                .query_row(
                    "SELECT name FROM friend_groups WHERE id = ?1",
                    rusqlite::params![group_id],
                    |row| row.get(0),
                )
                .optional()?;
            Ok(name)
        })
        .await?;

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

/// Result of generating an invite: URL + tracking token.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateInviteResult {
    pub url: String,
    pub invite_id: String,
}

/// Generate an invite link containing everything needed for a peer to add us.
#[tauri::command]
pub async fn generate_invite(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<GenerateInviteResult, String> {
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

    // Wait up to 30s for DHT publish to complete (route_blob, profile key, mailbox key)
    let (profile_dht_key, route_blob, mailbox_dht_key) = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        async {
            loop {
                match state_helpers::profile_dht_info(state.inner()) {
                    Ok(info) => return info,
                    Err(_) => tokio::time::sleep(std::time::Duration::from_millis(500)).await,
                }
            }
        },
    )
    .await
    .map_err(|_| "Network not ready — please wait a moment and try again".to_string())?;

    tracing::info!(
        route_blob_len = route_blob.len(),
        route_count = route_blob.first().copied().unwrap_or(0),
        route_blob_hex_preview = %hex::encode(&route_blob[..route_blob.len().min(32)]),
        profile_dht_key = %profile_dht_key,
        mailbox_dht_key = %mailbox_dht_key,
        "generate_invite: route blob from state"
    );

    // Validate that our own route blob is importable (sanity check)
    if let Some(api) = state_helpers::veilid_api(state.inner()) {
        match api.import_remote_private_route(route_blob.clone()) {
            Ok(_) => tracing::info!("generate_invite: route blob self-import OK"),
            Err(e) => {
                tracing::error!(error = %e, "generate_invite: OUR OWN route blob fails to import!");
                return Err(format!("route blob is invalid: {e}"));
            }
        }
    }

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

    // Generate a unique invite_id
    let invite_id = uuid::Uuid::new_v4().to_string();

    let blob = rekindle_protocol::messaging::create_invite_blob(
        &secret_key,
        &public_key,
        &display_name,
        &mailbox_dht_key,
        &profile_dht_key,
        &route_blob,
        &prekey_bundle,
        Some(&invite_id),
    );
    let url = rekindle_protocol::messaging::encode_invite_url(&blob);

    // Track in the database (store URL so it can be re-copied later)
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    crate::invite_helpers::create_outgoing_invite(pool.inner(), &owner_key, &invite_id, &url)
        .await?;

    tracing::info!(%invite_id, "generated tracked invite");
    Ok(GenerateInviteResult { url, invite_id })
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

    let owner_key = state_helpers::current_owner_key(state.inner())?;

    // Prevent adding yourself
    if blob.public_key == owner_key {
        return Err("You cannot add yourself as a friend".to_string());
    }

    // Prevent adding a blocked user
    if is_user_blocked(pool.inner(), &owner_key, &blob.public_key).await {
        return Err("Cannot add a blocked user. Unblock them first.".to_string());
    }

    let timestamp = db::timestamp_now();

    // Clean up any stale state from a previous friendship (e.g., re-adding after removal).
    // The Removing grace period keeps in-memory state alive for unfriend retries,
    // but on re-add we must clear it so fresh connections are established.
    {
        let is_stale = state_helpers::is_friend(state.inner(), &blob.public_key);
        if is_stale {
            // Invalidate stale route cache so fresh route from invite is used
            let mut dht_mgr = state.dht_manager.write();
            if let Some(mgr) = dht_mgr.as_mut() {
                mgr.manager.invalidate_route_for_peer(&blob.public_key);
            }
            // Remove stale in-memory entry (the .insert() below will create fresh state)
            state.friends.write().remove(&blob.public_key);
        }
    }

    // Insert into SQLite with profile and mailbox keys from the invite
    // (remove_friend already DELETEd the old row; this creates a fresh one)
    let pk = blob.public_key.clone();
    let dn = blob.display_name.clone();
    let ok = owner_key;
    let profile_key = blob.profile_dht_key.clone();
    let mailbox_key = blob.mailbox_dht_key.clone();
    db_call(pool.inner(), move |conn| {
        // DELETE first to handle any leftover rows (e.g., race with grace period cleanup),
        // then INSERT fresh so all columns are populated from the invite.
        conn.execute(
            "DELETE FROM friends WHERE owner_key = ?1 AND public_key = ?2",
            rusqlite::params![ok, pk],
        )?;
        conn.execute(
            "INSERT INTO friends (owner_key, public_key, display_name, added_at, dht_record_key, mailbox_dht_key, friendship_state) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending_out')",
            rusqlite::params![ok, pk, dn, timestamp, profile_key, mailbox_key],
        )?;
        Ok(())
    })
    .await?;

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
    state
        .friends
        .write()
        .insert(blob.public_key.clone(), friend);

    // Cache route blob, establish Signal session, and try mailbox for fresh route
    setup_invite_contact(state.inner(), &blob).await;

    // Send friend request via Veilid (includes our profile key + route blob + mailbox key)
    services::message_service::send_friend_request(
        state.inner(),
        pool.inner(),
        &blob.public_key,
        "Added via invite link",
        blob.invite_id.as_deref(),
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
    tracing::info!(
        peer = %blob.public_key,
        route_blob_len = blob.route_blob.len(),
        route_count = blob.route_blob.first().copied().unwrap_or(0),
        route_blob_hex_preview = %hex::encode(&blob.route_blob[..blob.route_blob.len().min(32)]),
        "setup_invite_contact: received route blob from invite"
    );
    let api = state_helpers::veilid_api(state);
    if let Some(ref api) = api {
        let mut dht_mgr = state.dht_manager.write();
        if let Some(mgr) = dht_mgr.as_mut() {
            mgr.manager
                .cache_route(api, &blob.public_key, blob.route_blob.clone());
        }
    } else {
        tracing::warn!("setup_invite_contact: no veilid API available — cannot cache route");
    }

    // Establish Signal session from invite's PreKeyBundle
    // Clear any stale session first (e.g., from a previous friendship that was removed)
    if let Ok(bundle) =
        serde_json::from_slice::<rekindle_crypto::signal::PreKeyBundle>(&blob.prekey_bundle)
    {
        let signal = state.signal_manager.lock();
        if let Some(handle) = signal.as_ref() {
            let _ = handle.manager.delete_session(&blob.public_key);
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
    let rc = state
        .node
        .read()
        .as_ref()
        .map(|nh| nh.routing_context.clone());
    if let Some(rc) = rc {
        match rekindle_protocol::dht::mailbox::read_peer_mailbox_route(&rc, &blob.mailbox_dht_key)
            .await
        {
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

/// A blocked user entry returned by `get_blocked_users`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockedUser {
    pub public_key: String,
    pub display_name: String,
    pub blocked_at: i64,
}

/// Check if a user is in the blocked list for the current identity.
pub(crate) async fn is_user_blocked(pool: &DbPool, owner_key: &str, public_key: &str) -> bool {
    let ok = owner_key.to_string();
    let pk = public_key.to_string();
    db_call_or_default(pool, move |conn| {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM blocked_users WHERE owner_key = ?1 AND public_key = ?2",
                rusqlite::params![ok, pk],
                |row| row.get(0),
            )
            .unwrap_or(0);
        Ok(count > 0)
    })
    .await
}

/// Block a user — works for any public key (friend, pending, invite, or raw key).
///
/// Removes them from friends/pending requests, adds to blocked list, cleans up
/// Signal session, pending messages, DHT state, and rotates our profile key.
#[tauri::command]
pub async fn block_user(
    public_key: String,
    display_name: Option<String>,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    let timestamp = db::timestamp_now();

    // Resolve display name: in-memory friends → pending requests store → param → truncated key
    let resolved_name = state_helpers::friend_display_name(state.inner(), &public_key)
        .or_else(|| display_name.clone())
        .unwrap_or_else(|| {
            let truncated = if public_key.len() > 12 {
                format!("{}...", &public_key[..12])
            } else {
                public_key.clone()
            };
            truncated
        });

    // DB transaction: DELETE from friends + pending_friend_requests + INSERT into blocked_users
    let pk = public_key.clone();
    let ok = owner_key;
    let dn = resolved_name;
    db_call(pool.inner(), move |conn| {
        conn.execute(
            "DELETE FROM friends WHERE owner_key = ?1 AND public_key = ?2",
            rusqlite::params![ok, pk],
        )?;
        conn.execute(
            "DELETE FROM pending_friend_requests WHERE owner_key = ?1 AND public_key = ?2",
            rusqlite::params![ok, pk],
        )?;
        conn.execute(
            "INSERT OR REPLACE INTO blocked_users (owner_key, public_key, display_name, blocked_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![ok, pk, dn, timestamp],
        )?;
        Ok(())
    })
    .await?;

    // Delete queued pending_messages to this user
    services::message_service::delete_pending_messages_to_recipient(
        state.inner(),
        pool.inner(),
        &public_key,
    );

    // Remove from in-memory state and unregister DHT key mapping + invalidate route
    let dht_key = {
        let mut friends = state.friends.write();
        let removed = friends.remove(&public_key);
        removed.and_then(|f| f.dht_record_key)
    };
    {
        let mut dht_mgr = state.dht_manager.write();
        if let Some(mgr) = dht_mgr.as_mut() {
            if let Some(ref dht_key) = dht_key {
                mgr.unregister_friend_dht_key(dht_key);
            }
            mgr.manager.invalidate_route_for_peer(&public_key);
        }
    }

    // Delete Signal session for the blocked user
    {
        let signal = state.signal_manager.lock();
        if let Some(handle) = signal.as_ref() {
            let _ = handle.manager.delete_session(&public_key);
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

    tracing::info!(public_key = %public_key, "user blocked and profile key rotated");
    Ok(())
}

/// Unblock a user — removes them from the blocked list.
///
/// Does NOT re-add them as a friend. The user must manually re-add if desired.
#[tauri::command]
pub async fn unblock_user(
    public_key: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    let pk = public_key.clone();
    let ok = owner_key;
    db_call(pool.inner(), move |conn| {
        conn.execute(
            "DELETE FROM blocked_users WHERE owner_key = ?1 AND public_key = ?2",
            rusqlite::params![ok, pk],
        )?;
        Ok(())
    })
    .await?;

    tracing::info!(public_key = %public_key, "user unblocked");
    Ok(())
}

/// Get all blocked users for the current identity.
#[tauri::command]
pub async fn get_blocked_users(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<Vec<BlockedUser>, String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    db_call(pool.inner(), move |conn| {
        let mut stmt = conn.prepare(
            "SELECT public_key, display_name, blocked_at \
             FROM blocked_users WHERE owner_key = ?1 ORDER BY blocked_at DESC",
        )?;
        let rows = stmt.query_map(rusqlite::params![owner_key], |row| {
            Ok(BlockedUser {
                public_key: row.get(0)?,
                display_name: row.get(1)?,
                blocked_at: row.get(2)?,
            })
        })?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    })
    .await
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
    let nk = new_key.clone();
    let keypair_str = new_keypair.as_ref().map(std::string::ToString::to_string);
    let owner_key = state_helpers::owner_key_or_default(state);
    db_call(pool, move |conn| {
        conn.execute(
            "UPDATE identity SET dht_record_key = ?1, dht_owner_keypair = COALESCE(?3, dht_owner_keypair) WHERE public_key = ?2",
            rusqlite::params![nk, owner_key, keypair_str],
        )?;
        Ok(())
    })
    .await?;

    // Notify all remaining friends about the new profile key
    let friend_keys: Vec<String> = {
        let friends = state.friends.read();
        friends.keys().cloned().collect()
    };
    let payload = rekindle_protocol::messaging::envelope::MessagePayload::ProfileKeyRotated {
        new_profile_dht_key: new_key.clone(),
    };
    for fk in &friend_keys {
        if let Err(e) = services::message_service::send_to_peer_raw(state, pool, fk, &payload).await
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
        friends
            .values()
            .filter(|f| f.friendship_state == FriendshipState::Accepted)
            .map(|f| (f.public_key.clone(), f.status))
            .collect()
    };
    for (key, status) in friends {
        if status != UserStatus::Offline {
            // Emit FriendOnline first so the frontend transitions the friend
            // out of the offline visual group before updating the specific status.
            let _ = app.emit(
                "presence-event",
                &crate::channels::PresenceEvent::FriendOnline {
                    public_key: key.clone(),
                },
            );
            let _ = app.emit(
                "presence-event",
                &crate::channels::PresenceEvent::StatusChanged {
                    public_key: key,
                    status: format!("{status:?}").to_lowercase(),
                    status_message: None,
                },
            );
        }
    }
    Ok(())
}

/// Cancel a pending outgoing invite by its `invite_id`.
#[tauri::command]
pub async fn cancel_invite(
    invite_id: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    crate::invite_helpers::cancel_outgoing_invite(pool.inner(), &owner_key, &invite_id).await?;
    tracing::info!(%invite_id, "invite cancelled");
    Ok(())
}

/// Get all active (pending/responded) outgoing invites.
#[tauri::command]
pub async fn get_outgoing_invites(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<Vec<crate::invite_helpers::OutgoingInvite>, String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    crate::invite_helpers::get_pending_invites(pool.inner(), &owner_key).await
}
