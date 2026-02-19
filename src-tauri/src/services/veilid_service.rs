use std::sync::Arc;

use tauri::{Emitter, Manager};
use tokio::sync::mpsc;
use veilid_core::VeilidUpdate;

use crate::channels::{NetworkStatusEvent, NotificationEvent};
use crate::db::DbPool;
use crate::state::{
    AppState, DHTManagerHandle, NodeHandle, RoutingManagerHandle,
};

/// Build and emit a `NetworkStatusEvent` from current `NodeHandle` state.
///
/// Called from any code path that changes attachment, readiness, or route status
/// so the frontend's `NetworkIndicator` updates instantly.
pub fn emit_network_status(app_handle: &tauri::AppHandle, state: &AppState) {
    let event = {
        let node = state.node.read();
        match node.as_ref() {
            Some(nh) => NetworkStatusEvent {
                attachment_state: nh.attachment_state.clone(),
                is_attached: nh.is_attached,
                public_internet_ready: nh.public_internet_ready,
                has_route: nh.route_blob.is_some(),
            },
            None => NetworkStatusEvent {
                attachment_state: "detached".to_string(),
                is_attached: false,
                public_internet_ready: false,
                has_route: false,
            },
        }
    };
    let _ = app_handle.emit("network-status", &event);
}

/// Start the Veilid event dispatch loop.
///
/// This is the heartbeat of the application. It receives real `VeilidUpdate`
/// events from the node's internal callback channel and routes them to
/// the appropriate service handler.
pub async fn start_dispatch_loop(
    app_handle: tauri::AppHandle,
    state: Arc<AppState>,
    mut update_rx: mpsc::Receiver<VeilidUpdate>,
    mut shutdown_rx: mpsc::Receiver<()>,
) {
    tracing::info!("veilid dispatch loop started");

    loop {
        tokio::select! {
            Some(update) = update_rx.recv() => {
                handle_veilid_update(&app_handle, &state, update).await;
            }
            _ = shutdown_rx.recv() => {
                tracing::info!("veilid dispatch loop shutting down");
                break;
            }
        }
    }
}

/// Route a single `VeilidUpdate` to the appropriate handler.
async fn handle_veilid_update(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    update: VeilidUpdate,
) {
    match update {
        VeilidUpdate::AppMessage(msg) => handle_app_message(app_handle, state, *msg).await,
        VeilidUpdate::AppCall(call) => handle_app_call(app_handle, state, *call).await,
        VeilidUpdate::ValueChange(change) => {
            handle_value_change(app_handle, state, *change).await;
        }
        VeilidUpdate::Attachment(attachment) => {
            handle_attachment(app_handle, state, &attachment);
        }
        VeilidUpdate::RouteChange(change) => {
            handle_route_change(app_handle, state, &change).await;
        }
        VeilidUpdate::Shutdown => {
            tracing::info!("veilid core shutdown event received");
        }
        // Log, Network, Config updates are informational
        _ => {}
    }
}

/// Handle an incoming `AppMessage` by routing it through the message service.
///
/// Routing order:
/// 1. Voice packets (prefixed with `b'V'`) → voice engine receive channel
/// 2. Community broadcasts (JSON) → community handler
/// 3. Everything else → standard message envelope handling
async fn handle_app_message(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    msg: veilid_core::VeilidAppMessage,
) {
    let message = msg.message().to_vec();
    tracing::debug!(msg_len = message.len(), "app_message received");

    // 1. Check for voice packet (tagged with b'V' prefix)
    if !message.is_empty() && message[0] == b'V' {
        let voice_data = &message[1..];
        match rekindle_voice::transport::VoiceTransport::receive(voice_data) {
            Ok(packet) => {
                let tx = state.voice_packet_tx.read().clone();
                if let Some(tx) = tx {
                    if tx.try_send(packet).is_err() {
                        tracing::trace!("voice packet channel full or closed — dropping packet");
                    }
                }
            }
            Err(e) => {
                tracing::trace!(error = %e, "failed to deserialize voice packet");
            }
        }
        return;
    }

    // 2. Try to parse as a community broadcast
    if let Ok(broadcast) = serde_json::from_slice::<rekindle_protocol::messaging::CommunityBroadcast>(&message) {
        handle_community_broadcast(app_handle, state, broadcast).await;
        return;
    }

    // 3. Fallback to standard message handling
    let pool: tauri::State<'_, DbPool> = app_handle.state();
    super::message_service::handle_incoming_message(
        app_handle,
        state,
        pool.inner(),
        &message,
    )
    .await;
}

/// Handle a community broadcast from the community server.
async fn handle_community_broadcast(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    broadcast: rekindle_protocol::messaging::CommunityBroadcast,
) {
    use rekindle_protocol::messaging::CommunityBroadcast;

    match broadcast {
        CommunityBroadcast::NewMessage {
            community_id,
            channel_id,
            sender_pseudonym,
            ciphertext,
            mek_generation,
            timestamp,
        } => {
            let msg = BroadcastNewMessage {
                community_id, channel_id, sender_pseudonym,
                ciphertext, mek_generation, timestamp,
            };
            handle_broadcast_new_message(app_handle, state, &msg).await;
        }
        CommunityBroadcast::MEKRotated {
            community_id,
            new_generation,
        } => {
            handle_broadcast_mek_rotated(app_handle, state, &community_id, new_generation).await;
        }
        CommunityBroadcast::MemberJoined {
            community_id,
            pseudonym_key,
            display_name,
            role_ids,
        } => {
            handle_broadcast_member_joined(
                app_handle, state, &community_id, &pseudonym_key, &display_name, &role_ids,
            ).await;
        }
        CommunityBroadcast::MemberRemoved {
            community_id,
            pseudonym_key,
        } => {
            handle_broadcast_member_removed(app_handle, state, &community_id, &pseudonym_key).await;
        }
        CommunityBroadcast::RolesChanged {
            community_id,
            roles,
        } => {
            handle_broadcast_roles_changed(app_handle, state, &community_id, roles).await;
        }
        CommunityBroadcast::MemberRolesChanged {
            community_id,
            pseudonym_key,
            role_ids,
        } => {
            handle_broadcast_member_roles_changed(app_handle, state, &community_id, &pseudonym_key, &role_ids).await;
        }
        CommunityBroadcast::MemberTimedOut {
            community_id,
            pseudonym_key,
            timeout_until,
        } => {
            handle_broadcast_member_timed_out(app_handle, state, &community_id, &pseudonym_key, timeout_until).await;
        }
        CommunityBroadcast::ChannelOverwriteChanged {
            community_id,
            channel_id,
        } => {
            tracing::info!(
                community = %community_id,
                channel = %channel_id,
                "channel overwrite changed — permission enforcement is server-side; \
                 client will see updated permissions on next channel interaction"
            );
            // Channel permission overwrites are enforced server-side.
            // The client sends SetChannelOverwrite / DeleteChannelOverwrite RPCs
            // and the server evaluates effective permissions for every action.
            // A GetChannelOverwrites RPC would be needed to cache them client-side
            // for UI display. For now, we emit an event so the UI can show a refresh hint.
            let event = crate::channels::CommunityEvent::ChannelOverwriteChanged {
                community_id,
                channel_id,
            };
            let _ = app_handle.emit("community-event", &event);
        }
    }
}

/// Decrypt result for community message MEK decryption attempts.
enum MekDecryptResult {
    Decrypted(String),
    NeedRefresh,
    Failed,
}

/// Parameters for handling a new community message broadcast.
struct BroadcastNewMessage {
    community_id: String,
    channel_id: String,
    sender_pseudonym: String,
    ciphertext: Vec<u8>,
    mek_generation: u64,
    timestamp: u64,
}

/// Handle a `NewMessage` community broadcast: decrypt, store, and emit.
async fn handle_broadcast_new_message(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    msg: &BroadcastNewMessage,
) {
    // Skip messages we sent ourselves (already echoed locally in send_channel_message)
    {
        let communities = state.communities.read();
        if let Some(community) = communities.get(&msg.community_id) {
            if community.my_pseudonym_key.as_deref() == Some(&msg.sender_pseudonym) {
                return;
            }
        }
    }

    let first_attempt = {
        let mek_cache = state.mek_cache.lock();
        decrypt_with_cached_mek(&mek_cache, &msg.community_id, &msg.ciphertext, msg.mek_generation)
    }; // guard dropped here — safe to .await

    let body = match first_attempt {
        MekDecryptResult::Decrypted(body) => body,
        MekDecryptResult::Failed => return,
        MekDecryptResult::NeedRefresh => {
            fetch_mek_from_server(app_handle, state, &msg.community_id).await;

            // Retry with refreshed MEK
            let mek_cache = state.mek_cache.lock();
            if let MekDecryptResult::Decrypted(body) = decrypt_with_cached_mek(&mek_cache, &msg.community_id, &msg.ciphertext, msg.mek_generation) {
                body
            } else {
                tracing::warn!("MEK still mismatched after refresh — dropping message");
                return;
            }
        }
    };

    // Store locally
    let owner_key = state
        .identity
        .read()
        .as_ref()
        .map(|id| id.public_key.clone())
        .unwrap_or_default();

    let pool: tauri::State<'_, crate::db::DbPool> = app_handle.state();
    let pool = pool.inner().clone();
    let cid = msg.channel_id.clone();
    let spn = msg.sender_pseudonym.clone();
    let body_text = body.clone();
    let ts = msg.timestamp.cast_signed();
    let mg = msg.mek_generation.cast_signed();
    let _ = tokio::task::spawn_blocking(move || {
        let conn = pool.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO messages (owner_key, conversation_id, conversation_type, sender_key, body, timestamp, is_read, mek_generation) \
             VALUES (?, ?, 'channel', ?, ?, ?, 0, ?)",
            rusqlite::params![owner_key, cid, spn, body_text, ts, mg],
        )
        .map_err(|e| e.to_string())
    })
    .await;

    // Emit to frontend
    let event = crate::channels::ChatEvent::MessageReceived {
        from: msg.sender_pseudonym.clone(),
        body,
        timestamp: msg.timestamp,
        conversation_id: msg.channel_id.clone(),
    };
    let _ = app_handle.emit("chat-event", &event);
}

/// Try to decrypt ciphertext using the cached MEK for a community.
fn decrypt_with_cached_mek(
    mek_cache: &std::collections::HashMap<String, rekindle_crypto::group::media_key::MediaEncryptionKey>,
    community_id: &str,
    ciphertext: &[u8],
    mek_generation: u64,
) -> MekDecryptResult {
    match mek_cache.get(community_id) {
        Some(mek) if mek.generation() == mek_generation => {
            match mek.decrypt(ciphertext) {
                Ok(plaintext) => MekDecryptResult::Decrypted(
                    String::from_utf8(plaintext).unwrap_or_default(),
                ),
                Err(e) => {
                    tracing::warn!(error = %e, "failed to decrypt community message");
                    MekDecryptResult::Failed
                }
            }
        }
        Some(mek) => {
            tracing::warn!(
                have = mek.generation(),
                need = mek_generation,
                "MEK generation mismatch — fetching updated MEK from server"
            );
            MekDecryptResult::NeedRefresh
        }
        None => {
            tracing::warn!(community = %community_id, "no MEK cached for community");
            MekDecryptResult::Failed
        }
    }
}

/// Handle a `MEKRotated` community broadcast.
async fn handle_broadcast_mek_rotated(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    new_generation: u64,
) {
    tracing::info!(
        community = %community_id,
        generation = new_generation,
        "MEK rotated — fetching new key from server"
    );
    fetch_mek_from_server(app_handle, state, community_id).await;

    let event = crate::channels::CommunityEvent::MekRotated {
        community_id: community_id.to_string(),
        new_generation,
    };
    let _ = app_handle.emit("community-event", &event);
}

/// Handle a `MemberJoined` community broadcast: persist and notify.
async fn handle_broadcast_member_joined(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    pseudonym_key: &str,
    display_name: &str,
    role_ids: &[u32],
) {
    tracing::info!(
        community = %community_id,
        member = %pseudonym_key,
        "member joined community"
    );
    let owner_key = state
        .identity
        .read()
        .as_ref()
        .map(|id| id.public_key.clone())
        .unwrap_or_default();
    let pool: tauri::State<'_, crate::db::DbPool> = app_handle.state();
    let pool = pool.inner().clone();
    let cid = community_id.to_string();
    let pk = pseudonym_key.to_string();
    let dn = display_name.to_string();
    let role_ids_json = serde_json::to_string(role_ids).unwrap_or_else(|_| "[0,1]".to_string());
    let now = crate::db::timestamp_now();
    let _ = tokio::task::spawn_blocking(move || {
        let conn = pool.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT OR IGNORE INTO community_members (owner_key, community_id, pseudonym_key, display_name, role_ids, joined_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
            rusqlite::params![owner_key, cid, pk, dn, role_ids_json, now],
        )
        .map_err(|e| e.to_string())
    })
    .await;

    let event = crate::channels::CommunityEvent::MemberJoined {
        community_id: community_id.to_string(),
        pseudonym_key: pseudonym_key.to_string(),
        display_name: display_name.to_string(),
        role_ids: role_ids.to_vec(),
    };
    let _ = app_handle.emit("community-event", &event);
}

/// Handle a `MemberRemoved` community broadcast: delete and notify.
///
/// If the removed member is US (our pseudonym), clean up local state
/// (remove community, clear MEK) and emit a Kicked event.
async fn handle_broadcast_member_removed(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    pseudonym_key: &str,
) {
    // Check if this is a self-removal (we were kicked)
    let is_self = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .and_then(|c| c.my_pseudonym_key.as_deref())
            .is_some_and(|pk| pk == pseudonym_key)
    };

    if is_self {
        tracing::warn!(community = %community_id, "we were kicked from community");

        // Clear MEK from cache
        state.mek_cache.lock().remove(community_id);

        // Remove MEK from Stronghold
        {
            use rekindle_crypto::keychain::{mek_key_name, VAULT_COMMUNITIES};
            use rekindle_crypto::Keychain as _;

            let ks_handle: tauri::State<'_, crate::keystore::KeystoreHandle> = app_handle.state();
            let ks = ks_handle.lock();
            if let Some(ref keystore) = *ks {
                let key_name = mek_key_name(community_id);
                if let Err(e) = keystore.delete_key(VAULT_COMMUNITIES, &key_name) {
                    tracing::warn!(error = %e, "failed to remove MEK from Stronghold after kick");
                }
            }
        }

        // Remove community from in-memory state
        state.communities.write().remove(community_id);

        // Remove from SQLite
        let owner_key = state
            .identity
            .read()
            .as_ref()
            .map(|id| id.public_key.clone())
            .unwrap_or_default();
        let pool: tauri::State<'_, crate::db::DbPool> = app_handle.state();
        let pool = pool.inner().clone();
        let cid = community_id.to_string();
        let _ = tokio::task::spawn_blocking(move || {
            let conn = pool.lock().map_err(|e| e.to_string())?;
            conn.execute(
                "DELETE FROM communities WHERE owner_key = ? AND id = ?",
                rusqlite::params![owner_key, cid],
            )
            .map_err(|e| e.to_string())
        })
        .await;

        // Emit kicked event to frontend
        let event = crate::channels::CommunityEvent::Kicked {
            community_id: community_id.to_string(),
        };
        let _ = app_handle.emit("community-event", &event);
        return;
    }

    tracing::info!(
        community = %community_id,
        member = %pseudonym_key,
        "member removed from community"
    );
    let owner_key = state
        .identity
        .read()
        .as_ref()
        .map(|id| id.public_key.clone())
        .unwrap_or_default();
    let pool: tauri::State<'_, crate::db::DbPool> = app_handle.state();
    let pool = pool.inner().clone();
    let cid = community_id.to_string();
    let pk = pseudonym_key.to_string();
    let _ = tokio::task::spawn_blocking(move || {
        let conn = pool.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM community_members WHERE owner_key = ? AND community_id = ? AND pseudonym_key = ?",
            rusqlite::params![owner_key, cid, pk],
        )
        .map_err(|e| e.to_string())
    })
    .await;

    let event = crate::channels::CommunityEvent::MemberRemoved {
        community_id: community_id.to_string(),
        pseudonym_key: pseudonym_key.to_string(),
    };
    let _ = app_handle.emit("community-event", &event);
}

/// Handle a `RolesChanged` community broadcast: update state, persist, notify.
async fn handle_broadcast_roles_changed(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    roles: Vec<rekindle_protocol::messaging::RoleDto>,
) {
    tracing::info!(
        community = %community_id,
        count = roles.len(),
        "community roles changed"
    );

    let role_defs: Vec<crate::state::RoleDefinition> =
        roles.iter().map(crate::state::RoleDefinition::from_dto).collect();

    // Update in-memory state and recompute our display role
    {
        let mut communities = state.communities.write();
        if let Some(c) = communities.get_mut(community_id) {
            c.roles.clone_from(&role_defs);
            c.my_role = Some(crate::state::display_role_name(&c.my_role_ids, &c.roles));
        }
    }

    // Persist to community_roles table
    let owner_key = state
        .identity
        .read()
        .as_ref()
        .map(|id| id.public_key.clone())
        .unwrap_or_default();
    let pool: tauri::State<'_, crate::db::DbPool> = app_handle.state();
    let pool = pool.inner().clone();
    let cid = community_id.to_string();
    let roles_clone = role_defs.clone();
    let _ = tokio::task::spawn_blocking(move || {
        let conn = pool.lock().map_err(|e| e.to_string())?;
        // Replace all roles for this community
        conn.execute(
            "DELETE FROM community_roles WHERE owner_key = ? AND community_id = ?",
            rusqlite::params![owner_key, cid],
        )
        .map_err(|e| e.to_string())?;
        for r in &roles_clone {
            conn.execute(
                "INSERT INTO community_roles (owner_key, community_id, role_id, name, color, permissions, position, hoist, mentionable) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    owner_key,
                    cid,
                    r.id,
                    r.name,
                    r.color,
                    r.permissions.cast_signed(),
                    r.position,
                    i32::from(r.hoist),
                    i32::from(r.mentionable),
                ],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok::<(), String>(())
    })
    .await;

    // Emit event to frontend
    let event = crate::channels::CommunityEvent::RolesChanged {
        community_id: community_id.to_string(),
        roles: role_defs
            .iter()
            .map(crate::channels::community_channel::RoleDto::from)
            .collect(),
    };
    let _ = app_handle.emit("community-event", &event);
}

/// Handle a `MemberRolesChanged` community broadcast: update state, persist, notify.
async fn handle_broadcast_member_roles_changed(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    pseudonym_key: &str,
    role_ids: &[u32],
) {
    tracing::info!(
        community = %community_id,
        member = %pseudonym_key,
        ?role_ids,
        "member roles changed"
    );

    // Check if this is us — update our my_role_ids
    {
        let mut communities = state.communities.write();
        if let Some(c) = communities.get_mut(community_id) {
            let is_self = c
                .my_pseudonym_key
                .as_deref()
                .is_some_and(|pk| pk == pseudonym_key);
            if is_self {
                c.my_role_ids = role_ids.to_vec();
                c.my_role = Some(crate::state::display_role_name(&c.my_role_ids, &c.roles));
            }
        }
    }

    // Persist to community_members table
    let owner_key = state
        .identity
        .read()
        .as_ref()
        .map(|id| id.public_key.clone())
        .unwrap_or_default();

    // Check if this is us before the first spawn_blocking (need owner_key available for both)
    let is_self = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .and_then(|c| c.my_pseudonym_key.as_deref())
            .is_some_and(|pk| pk == pseudonym_key)
    };

    let pool: tauri::State<'_, crate::db::DbPool> = app_handle.state();
    let pool_clone = pool.inner().clone();
    let cid = community_id.to_string();
    let pk = pseudonym_key.to_string();
    let ok = owner_key.clone();
    let role_ids_json = serde_json::to_string(role_ids).unwrap_or_else(|_| "[0,1]".to_string());
    let _ = tokio::task::spawn_blocking(move || {
        let conn = pool_clone.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE community_members SET role_ids = ? WHERE owner_key = ? AND community_id = ? AND pseudonym_key = ?",
            rusqlite::params![role_ids_json, ok, cid, pk],
        )
        .map_err(|e| e.to_string())
    })
    .await;

    // Also update our my_role_ids in the communities table if this is us
    if is_self {
        let pool_clone2 = pool.inner().clone();
        let cid2 = community_id.to_string();
        let ok2 = owner_key;
        let rids = serde_json::to_string(role_ids).unwrap_or_else(|_| "[0,1]".to_string());
        let _ = tokio::task::spawn_blocking(move || {
            let conn = pool_clone2.lock().map_err(|e| e.to_string())?;
            conn.execute(
                "UPDATE communities SET my_role_ids = ? WHERE owner_key = ? AND id = ?",
                rusqlite::params![rids, ok2, cid2],
            )
            .map_err(|e| e.to_string())
        })
        .await;
    }

    let event = crate::channels::CommunityEvent::MemberRolesChanged {
        community_id: community_id.to_string(),
        pseudonym_key: pseudonym_key.to_string(),
        role_ids: role_ids.to_vec(),
    };
    let _ = app_handle.emit("community-event", &event);
}

/// Handle a `MemberTimedOut` community broadcast: update state, persist, notify.
async fn handle_broadcast_member_timed_out(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    pseudonym_key: &str,
    timeout_until: Option<u64>,
) {
    tracing::info!(
        community = %community_id,
        member = %pseudonym_key,
        ?timeout_until,
        "member timeout changed"
    );

    // Persist to community_members table
    let owner_key = state
        .identity
        .read()
        .as_ref()
        .map(|id| id.public_key.clone())
        .unwrap_or_default();
    let pool: tauri::State<'_, crate::db::DbPool> = app_handle.state();
    let pool = pool.inner().clone();
    let cid = community_id.to_string();
    let pk = pseudonym_key.to_string();
    let timeout_i64 = timeout_until.map(u64::cast_signed);
    let _ = tokio::task::spawn_blocking(move || {
        let conn = pool.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE community_members SET timeout_until = ? WHERE owner_key = ? AND community_id = ? AND pseudonym_key = ?",
            rusqlite::params![timeout_i64, owner_key, cid, pk],
        )
        .map_err(|e| e.to_string())
    })
    .await;

    let event = crate::channels::CommunityEvent::MemberTimedOut {
        community_id: community_id.to_string(),
        pseudonym_key: pseudonym_key.to_string(),
        timeout_until,
    };
    let _ = app_handle.emit("community-event", &event);
}

/// Persist MEK to in-memory state, Stronghold, and the database.
async fn persist_mek(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    mek_generation: u64,
    mek_encrypted: &[u8],
) {
    // Update generation in community state
    {
        let mut communities = state.communities.write();
        if let Some(c) = communities.get_mut(community_id) {
            c.mek_generation = mek_generation;
        }
    }

    // Persist to Stronghold
    {
        use rekindle_crypto::keychain::{mek_key_name, VAULT_COMMUNITIES};
        use rekindle_crypto::Keychain as _;

        let ks_handle: tauri::State<'_, crate::keystore::KeystoreHandle> = app_handle.state();
        let ks = ks_handle.lock();
        if let Some(ref keystore) = *ks {
            let key_name = mek_key_name(community_id);
            if let Err(e) = keystore.store_key(VAULT_COMMUNITIES, &key_name, mek_encrypted) {
                tracing::warn!(error = %e, "failed to persist refreshed MEK to Stronghold");
            } else if let Err(e) = keystore.save() {
                tracing::warn!(error = %e, "failed to save Stronghold snapshot after MEK refresh");
            }
        }
    }

    // Persist mek_generation to SQLite
    {
        let pool: tauri::State<'_, crate::db::DbPool> = app_handle.state();
        let pool = pool.inner().clone();
        let cid = community_id.to_string();
        let owner_key = state
            .identity
            .read()
            .as_ref()
            .map(|id| id.public_key.clone())
            .unwrap_or_default();
        let gen_i64 = i64::try_from(mek_generation).unwrap_or(i64::MAX);
        let _ = tokio::task::spawn_blocking(move || {
            if let Ok(conn) = pool.lock() {
                let _ = conn.execute(
                    "UPDATE communities SET mek_generation = ? WHERE owner_key = ? AND id = ?",
                    rusqlite::params![gen_i64, owner_key, cid],
                );
            }
        })
        .await;
    }

    tracing::info!(
        community = %community_id,
        generation = mek_generation,
        "MEK persisted to state, Stronghold, and SQLite"
    );
}

/// Fetch the current MEK from the community server via `RequestMEK` RPC.
///
/// Updates `mek_cache` and community state on success. Also persists the
/// updated MEK to Stronghold so it survives restarts.
pub(super) async fn fetch_mek_from_server(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
) {
    let server_route_blob = {
        let communities = state.communities.read();
        communities.get(community_id).and_then(|c| c.server_route_blob.clone())
    };

    let node_info = {
        let node = state.node.read();
        node.as_ref()
            .filter(|n| n.is_attached)
            .map(|nh| (nh.routing_context.clone(), nh.api.clone()))
    };

    let signing_key = {
        let secret = state.identity_secret.lock();
        secret.map(|s| {
            rekindle_crypto::group::pseudonym::derive_community_pseudonym(&s, community_id)
        })
    };

    let (Some(route_blob), Some((rc, api)), Some(sk)) =
        (server_route_blob, node_info, signing_key)
    else {
        tracing::warn!(community = %community_id, "cannot fetch MEK — missing route/node/key");
        return;
    };

    let request = rekindle_protocol::messaging::CommunityRequest::RequestMEK;
    let Ok(request_bytes) = serde_json::to_vec(&request) else {
        return;
    };

    let timestamp = crate::db::timestamp_now().cast_unsigned();
    let mut nonce = vec![0u8; 24];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut nonce);
    let envelope = rekindle_protocol::messaging::sender::build_envelope(
        &sk, timestamp, nonce, request_bytes,
    );

    let route_id = {
        let mut dht_mgr = state.dht_manager.write();
        if let Some(mgr) = dht_mgr.as_mut() {
            match mgr.manager.get_or_import_route(&api, &route_blob) {
                Ok(rid) => rid,
                Err(e) => {
                    tracing::warn!(community = %community_id, error = %e, "failed to import server route for MEK fetch");
                    return;
                }
            }
        } else {
            let Ok(rid) = api.import_remote_private_route(route_blob) else {
                tracing::warn!(community = %community_id, "failed to import server route for MEK fetch");
                return;
            };
            rid
        }
    };

    let call_result = rekindle_protocol::messaging::sender::send_call(&rc, route_id, &envelope).await;

    match call_result {
        Ok(response_bytes) => {
            if let Ok(rekindle_protocol::messaging::CommunityResponse::MEK {
                mek_encrypted,
                mek_generation,
            }) = serde_json::from_slice(&response_bytes)
            {
                if mek_encrypted.len() >= 40 {
                    let key_bytes: [u8; 32] = mek_encrypted[8..40]
                        .try_into()
                        .unwrap_or_default();
                    let mek = rekindle_crypto::group::media_key::MediaEncryptionKey::from_bytes(
                        key_bytes, mek_generation,
                    );
                    state.mek_cache.lock().insert(community_id.to_string(), mek);
                    persist_mek(app_handle, state, community_id, mek_generation, &mek_encrypted).await;
                }
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, community = %community_id, "failed to fetch MEK from server");
        }
    }
}

/// Handle an incoming `AppCall` — process the message, then reply with ACK.
async fn handle_app_call(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    call: veilid_core::VeilidAppCall,
) {
    let call_id = call.id();
    tracing::debug!(call_id = %call_id, "app_call received");

    // Route the call through message handling (same as app_message)
    // then reply with an acknowledgment
    let message = call.message().to_vec();
    let pool: tauri::State<'_, DbPool> = app_handle.state();
    super::message_service::handle_incoming_message(
        app_handle,
        state,
        pool.inner(),
        &message,
    )
    .await;

    // Reply with ACK so the caller's app_call future resolves.
    // Clone the API handle outside the lock (parking_lot guards are !Send).
    let api = {
        let node = state.node.read();
        node.as_ref().map(|nh| nh.api.clone())
    };
    if let Some(api) = api {
        if let Err(e) = api.app_call_reply(call_id, b"ACK".to_vec()).await {
            tracing::warn!(error = %e, "failed to reply to app_call");
        }
    }
}

/// Handle a DHT `ValueChange` notification by forwarding to the presence service.
///
/// When the inline value is `None` (Veilid doesn't always include it), we fetch
/// each changed subkey's value from DHT individually.  The previous code silently
/// passed an empty vec which caused `parse_status` to return `None`, dropping
/// the status change entirely — this was why automated status updates (auto-away,
/// offline on logout) weren't visible to friends.
async fn handle_value_change(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    change: veilid_core::VeilidValueChange,
) {
    let key = change.key.to_string();

    // Detect watch death: empty subkeys means the watch has died.
    // Per veilid-core VeilidValueChange docs: "If the subkey range is empty,
    // any watch present on the value has died."
    if change.subkeys.is_empty() {
        tracing::warn!(
            key = %key, count = change.count,
            "DHT watch died — moving friend to poll fallback"
        );
        let friend_key = {
            let dht_mgr = state.dht_manager.read();
            dht_mgr
                .as_ref()
                .and_then(|mgr| mgr.friend_for_dht_key(&key).cloned())
        };
        if let Some(fk) = friend_key {
            state.unwatched_friends.write().insert(fk);
        }
        return;
    }

    // count == 0 with non-empty subkeys means this is the last change notification
    // before the watch expires. Process the change AND schedule a re-watch.
    if change.count == 0 {
        tracing::info!(
            key = %key,
            "DHT watch expiring (count=0) — will re-establish on next sync tick"
        );
        let friend_key = {
            let dht_mgr = state.dht_manager.read();
            dht_mgr
                .as_ref()
                .and_then(|mgr| mgr.friend_for_dht_key(&key).cloned())
        };
        if let Some(fk) = friend_key {
            state.unwatched_friends.write().insert(fk);
        }
        // Fall through to process the change below
    }

    let subkeys: Vec<u32> = change.subkeys.iter().collect();
    let inline_value = change.value.as_ref().map(|v| v.data().to_vec());
    tracing::debug!(
        key = %key,
        subkeys = ?subkeys,
        has_inline = inline_value.is_some(),
        "DHT value changed"
    );

    // Get routing context for fetching subkey values when not provided inline
    let routing_context = {
        let node = state.node.read();
        node.as_ref().map(|nh| nh.routing_context.clone())
    };

    for &subkey in &subkeys {
        let value = if let Some(ref v) = inline_value {
            v.clone()
        } else if let Some(ref rc) = routing_context {
            match rc
                .get_dht_value(change.key.clone(), subkey, true)
                .await
            {
                Ok(Some(v)) => v.data().to_vec(),
                Ok(None) => {
                    tracing::debug!(subkey, key = %key, "subkey has no value");
                    continue;
                }
                Err(e) => {
                    tracing::warn!(subkey, key = %key, error = %e, "failed to fetch subkey");
                    continue;
                }
            }
        } else {
            tracing::debug!(subkey, "no routing context to fetch subkey value");
            continue;
        };
        super::presence_service::handle_value_change(
            app_handle, state, &key, &[subkey], &value,
        )
        .await;
    }
}

/// Handle a network attachment state change — update node state and notify the frontend.
fn handle_attachment(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    attachment: &veilid_core::VeilidStateAttachment,
) {
    let attached = attachment.state.is_attached();
    let public_internet_ready = attachment.public_internet_ready;
    let state_str = attachment.state.to_string();
    tracing::info!(
        state = %state_str,
        public_internet_ready,
        "network attachment changed"
    );
    {
        if let Some(ref mut node) = *state.node.write() {
            node.attachment_state = state_str;
            node.is_attached = attached;
            node.public_internet_ready = public_internet_ready;
        }
    }
    // Propagate readiness via watch channel — never loses signals, no TOCTOU race
    let _ = state.network_ready_tx.send(public_internet_ready);

    // Push structured event so the frontend's NetworkIndicator can react immediately
    emit_network_status(app_handle, state);

    let status = if attached { "connected" } else { "disconnected" };
    let notification = NotificationEvent::SystemAlert {
        title: "Network".to_string(),
        body: format!("Veilid network {status}"),
    };
    let _ = app_handle.emit("notification-event", &notification);
}

/// Handle a route change — re-allocate our private route if it died, and
/// invalidate cached peer routes.
async fn handle_route_change(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    change: &veilid_core::VeilidRouteChange,
) {
    tracing::debug!(
        dead_routes = change.dead_routes.len(),
        dead_remote_routes = change.dead_remote_routes.len(),
        "route change event"
    );

    // Check if our specific private route died (not just any route)
    let our_route_died = {
        let rm = state.routing_manager.read();
        rm.as_ref().is_some_and(|handle| {
            handle
                .manager
                .route_id()
                .is_some_and(|our_id| change.dead_routes.contains(&our_id))
        })
    };

    if our_route_died {
        // The route is already dead — forget it (don't call release, which would
        // hit Veilid's "Invalid argument" error for an already-expired route).
        // Then allocate a fresh one.
        {
            let mut rm = state.routing_manager.write();
            if let Some(ref mut handle) = *rm {
                handle.manager.forget_private_route();
            }
        }
        allocate_fresh_private_route(app_handle, state).await;
    }

    // Invalidate cached peer routes that died (selective — only affected peers)
    if !change.dead_remote_routes.is_empty() {
        let mut dht_mgr = state.dht_manager.write();
        if let Some(mgr) = dht_mgr.as_mut() {
            mgr.manager
                .invalidate_dead_routes(&change.dead_remote_routes);
        }
    }
}

/// Allocate a new private route, then release the old one (make-before-break).
///
/// Used by the periodic refresh loop where the old route is still valid.
/// By allocating the new route FIRST, we avoid a window where `NodeHandle.route_blob`
/// holds a stale (released) blob. If allocation fails, the old route stays active.
pub(crate) async fn reallocate_private_route(app_handle: &tauri::AppHandle, state: &Arc<AppState>) {
    // Clone the API handle (Arc-based) outside the lock
    let api = {
        let node = state.node.read();
        node.as_ref().map(|nh| nh.api.clone())
    };
    let Some(api) = api else { return };

    // Allocate new route FIRST (make-before-break)
    let new_route = match api.new_private_route().await {
        Ok(rb) => rb,
        Err(e) => {
            tracing::warn!(error = %e, "route refresh: failed to allocate new route — keeping old");
            return;
        }
    };

    // New route allocated — NOW release the old one and store the new one
    {
        let mut rm = state.routing_manager.write();
        if let Some(ref mut handle) = *rm {
            if let Err(e) = handle.manager.release_private_route() {
                tracing::warn!(error = %e, "failed to release old private route");
            }
            handle.manager.set_allocated_route(
                new_route.route_id.clone(),
                new_route.blob.clone(),
            );
        }
    }
    // Update NodeHandle
    if let Some(ref mut nh) = *state.node.write() {
        nh.route_blob = Some(new_route.blob.clone());
    }

    // Notify frontend
    emit_network_status(app_handle, state);

    // Re-publish route blob to DHT profile subkey 6
    if let Err(e) =
        super::message_service::push_profile_update(state, 6, new_route.blob.clone()).await
    {
        tracing::warn!(error = %e, "failed to re-publish route blob to DHT");
    }

    // Also update mailbox subkey 0 with the fresh route blob
    let mailbox_key = {
        let node = state.node.read();
        node.as_ref().and_then(|nh| nh.mailbox_dht_key.clone())
    };
    if let Some(mailbox_key) = mailbox_key {
        let rc = {
            let node = state.node.read();
            node.as_ref().map(|nh| nh.routing_context.clone())
        };
        if let Some(rc) = rc {
            if let Err(e) = rekindle_protocol::dht::mailbox::update_mailbox_route(
                &rc,
                &mailbox_key,
                &new_route.blob,
            )
            .await
            {
                tracing::warn!(error = %e, "failed to update mailbox route blob");
            }
        }
    }

    tracing::info!("re-allocated private route (make-before-break)");
}

/// Allocate a fresh private route and publish it to DHT.
///
/// Assumes the old route has already been released or forgotten. Called by
/// both `reallocate_private_route` (periodic refresh) and `handle_route_change`
/// (dead route recovery).
async fn allocate_fresh_private_route(app_handle: &tauri::AppHandle, state: &Arc<AppState>) {
    // Clone the API handle (Arc-based) outside the lock
    let api = {
        let node = state.node.read();
        node.as_ref().map(|nh| nh.api.clone())
    };

    let Some(api) = api else {
        return;
    };

    match api.new_private_route().await {
        Ok(route_blob) => {
            // Store route info back in the routing manager
            {
                let mut rm = state.routing_manager.write();
                if let Some(ref mut handle) = *rm {
                    handle.manager.set_allocated_route(
                        route_blob.route_id.clone(),
                        route_blob.blob.clone(),
                    );
                }
            }
            // Also store on node handle
            if let Some(ref mut nh) = *state.node.write() {
                nh.route_blob = Some(route_blob.blob.clone());
            }
            // Notify the frontend immediately about the new route
            emit_network_status(app_handle, state);

            // Re-publish route blob to DHT profile subkey 6
            if let Err(e) =
                super::message_service::push_profile_update(state, 6, route_blob.blob.clone()).await
            {
                tracing::warn!(error = %e, "failed to re-publish route blob to DHT");
            }

            // Also update mailbox subkey 0 with the fresh route blob
            let mailbox_key = {
                let node = state.node.read();
                node.as_ref().and_then(|nh| nh.mailbox_dht_key.clone())
            };
            if let Some(mailbox_key) = mailbox_key {
                let rc = {
                    let node = state.node.read();
                    node.as_ref().map(|nh| nh.routing_context.clone())
                };
                if let Some(rc) = rc {
                    if let Err(e) = rekindle_protocol::dht::mailbox::update_mailbox_route(
                        &rc,
                        &mailbox_key,
                        &route_blob.blob,
                    )
                    .await
                    {
                        tracing::warn!(error = %e, "failed to update mailbox route blob");
                    }
                }
            }

            tracing::info!("re-allocated private route");
        }
        Err(e) => {
            tracing::warn!(error = %e, "failed to allocate private route");
        }
    }
}

/// Periodically re-allocate our private route to prevent silent expiration.
///
/// Veilid routes expire after ~5 minutes and `RouteChange` events can be missed.
/// This loop proactively re-allocates every 120 seconds to ensure peers can
/// always reach us. Spawned as a background task during login and aborted on logout.
pub(crate) async fn route_refresh_loop(
    app_handle: tauri::AppHandle,
    state: Arc<AppState>,
    mut shutdown_rx: mpsc::Receiver<()>,
) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(120));
    // Skip the immediate first tick (route was just allocated at login)
    interval.tick().await;

    loop {
        tokio::select! {
            _ = interval.tick() => {
                // Only refresh if we have a node and it's attached
                let should_refresh = {
                    let node = state.node.read();
                    node.as_ref().is_some_and(|nh| nh.is_attached && nh.route_blob.is_some())
                };
                if should_refresh {
                    tracing::debug!("proactive route refresh: re-allocating private route");
                    reallocate_private_route(&app_handle, &state).await;
                }
            }
            _ = shutdown_rx.recv() => {
                tracing::debug!("route refresh loop shutting down");
                break;
            }
        }
    }
}

/// Initialize the Veilid node (called once at app startup).
///
/// Starts the real Veilid node, attaches to the P2P network, creates
/// a routing context, and stores all handles in `AppState`. Returns the
/// `VeilidUpdate` receiver for the dispatch loop.
///
/// The node lives for the entire app lifetime — user login/logout does NOT
/// restart the node. Only `shutdown_app()` (on app exit) shuts it down.
pub async fn initialize_node(
    app_handle: &tauri::AppHandle,
    state: &AppState,
) -> Result<mpsc::Receiver<VeilidUpdate>, String> {
    // Determine storage directory inside the Tauri app data dir
    let app_data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| format!("failed to resolve app data dir: {e}"))?;
    let storage_dir = app_data_dir.join("veilid");
    std::fs::create_dir_all(&storage_dir)
        .map_err(|e| format!("failed to create veilid storage dir: {e}"))?;

    let config = rekindle_protocol::node::NodeConfig {
        storage_dir: storage_dir.to_string_lossy().into_owned(),
        app_namespace: "rekindle".into(),
    };

    // Start the real Veilid node (api_startup + attach + routing_context)
    let mut node = rekindle_protocol::RekindleNode::start(config)
        .await
        .map_err(|e| format!("failed to start veilid node: {e}"))?;

    // Take the VeilidUpdate receiver before storing the node's pieces
    let update_rx = node
        .take_update_receiver()
        .ok_or_else(|| "update receiver already taken".to_string())?;

    // Clone Arc-based handles before storing
    let api = node.api().clone();
    let routing_context = node.routing_context().clone();

    // Store NodeHandle in AppState
    // is_attached starts false — the dispatch loop will set it to true
    // when the first Attachment event with is_attached() arrives.
    let node_handle = NodeHandle {
        attachment_state: "detached".to_string(),
        is_attached: false,
        public_internet_ready: false,
        api: api.clone(),
        routing_context: routing_context.clone(),
        route_blob: None,
        profile_dht_key: None,
        profile_owner_keypair: None,
        friend_list_dht_key: None,
        friend_list_owner_keypair: None,
        account_dht_key: None,
        mailbox_dht_key: None,
    };
    *state.node.write() = Some(node_handle);

    // Create and store DHTManager
    let dht_handle = DHTManagerHandle::new(routing_context);
    *state.dht_manager.write() = Some(dht_handle);

    // Create and store RoutingManager (route allocation is deferred to
    // spawn_dht_publish() which waits for the network to be ready first)
    let routing_manager = rekindle_protocol::routing::RoutingManager::new(
        api,
        rekindle_protocol::routing::SafetyMode::default(),
    );
    *state.routing_manager.write() = Some(RoutingManagerHandle {
        manager: routing_manager,
    });

    tracing::info!("rekindle node started and attached");
    Ok(update_rx)
}

/// Clean up user-specific state on logout without shutting down the Veilid node.
///
/// The node stays alive for the entire app lifetime. This function:
/// 1. Aborts user-specific background tasks (sync, game detection, DHT publish)
/// 2. Closes all tracked DHT records
/// 3. Releases the private route
/// 4. Clears user-specific mappings from the DHT manager (but keeps the manager alive)
/// 5. Clears identity, friends, communities, signal manager
///
/// Does NOT call `api.shutdown()` — the node continues running for re-login.
#[allow(clippy::too_many_lines)]
pub async fn logout_cleanup(app_handle: Option<&tauri::AppHandle>, state: &AppState) {
    // 0. Shut down voice if active
    {
        let (send_tx, send_h, recv_tx, recv_h, monitor_tx, monitor_h) = {
            let mut ve = state.voice_engine.lock();
            if let Some(ref mut handle) = *ve {
                (
                    handle.send_loop_shutdown.take(),
                    handle.send_loop_handle.take(),
                    handle.recv_loop_shutdown.take(),
                    handle.recv_loop_handle.take(),
                    handle.device_monitor_shutdown.take(),
                    handle.device_monitor_handle.take(),
                )
            } else {
                (None, None, None, None, None, None)
            }
        };
        if let Some(tx) = send_tx { let _ = tx.send(()).await; }
        if let Some(tx) = recv_tx { let _ = tx.send(()).await; }
        if let Some(tx) = monitor_tx { let _ = tx.send(()).await; }
        if let Some(h) = send_h { let _ = h.await; }
        if let Some(h) = recv_h { let _ = h.await; }
        if let Some(h) = monitor_h { let _ = h.await; }
        {
            let mut ve = state.voice_engine.lock();
            if let Some(ref mut handle) = *ve {
                handle.engine.stop_capture();
                handle.engine.stop_playback();
            }
            *ve = None;
        }
        *state.voice_packet_tx.write() = None;
    }

    // Shut down idle service
    {
        let tx = state.idle_shutdown_tx.write().take();
        if let Some(tx) = tx {
            let _ = tx.send(()).await;
        }
    }
    *state.pre_away_status.write() = None;

    // Shut down heartbeat service
    {
        let tx = state.heartbeat_shutdown_tx.write().take();
        if let Some(tx) = tx {
            let _ = tx.send(()).await;
        }
    }

    // 1. Abort user-specific background tasks
    {
        let mut handles = state.background_handles.lock();
        for handle in handles.drain(..) {
            handle.abort();
        }
    }

    // 2. Close ALL open DHT records tracked during this session
    {
        let rc_and_keys = {
            let node = state.node.read();
            let rc = node.as_ref().map(|nh| nh.routing_context.clone());
            let keys: Vec<String> = {
                let dht_mgr = state.dht_manager.read();
                dht_mgr
                    .as_ref()
                    .map(|mgr| mgr.open_records.iter().cloned().collect())
                    .unwrap_or_default()
            };
            rc.map(|rc| (rc, keys))
        };
        if let Some((rc, keys)) = rc_and_keys {
            tracing::debug!(count = keys.len(), "closing open DHT records for logout");
            for key_str in &keys {
                if let Ok(record_key) = key_str.parse::<veilid_core::RecordKey>() {
                    if let Err(e) = rc.close_dht_record(record_key).await {
                        tracing::trace!(key = %key_str, error = %e, "close DHT record on logout");
                    }
                }
            }
        }
    }

    // 3. Release private route
    {
        let mut rm = state.routing_manager.write();
        if let Some(ref mut handle) = *rm {
            if let Err(e) = handle.manager.release_private_route() {
                tracing::warn!(error = %e, "failed to release private route during logout");
            }
        }
    }

    // 4. Clear user-specific state from DHT manager (keep manager alive for re-login)
    {
        let mut dht_mgr = state.dht_manager.write();
        if let Some(ref mut mgr) = *dht_mgr {
            mgr.dht_key_to_friend.clear();
            mgr.conversation_key_to_friend.clear();
            mgr.open_records.clear();
            mgr.manager.route_cache.clear();
            mgr.manager.imported_routes.clear();
            mgr.manager.route_id_to_pubkey.clear();
            mgr.manager.profile_key = None;
            mgr.manager.friend_list_key = None;
        }
    }

    // 5. Clear user-specific data from NodeHandle (keep node alive)
    {
        let mut node = state.node.write();
        if let Some(ref mut nh) = *node {
            nh.route_blob = None;
            nh.profile_dht_key = None;
            nh.profile_owner_keypair = None;
            nh.friend_list_dht_key = None;
            nh.friend_list_owner_keypair = None;
            nh.account_dht_key = None;
            nh.mailbox_dht_key = None;
        }
    }

    // Notify the frontend that the route is gone
    if let Some(ah) = app_handle {
        emit_network_status(ah, state);
    }

    // 6. Clear identity/friends/communities/signal
    *state.identity.write() = None;
    state.friends.write().clear();
    state.communities.write().clear();
    *state.signal_manager.lock() = None;
    *state.identity_secret.lock() = None;

    // 7. Clear community-specific state
    state.mek_cache.lock().clear();
    state.community_routes.write().clear();

    // 8. Shutdown server health check loop
    {
        let tx = state.server_health_shutdown_tx.write().take();
        if let Some(tx) = tx {
            let _ = tx.send(()).await;
        }
    }

    // 9. Shutdown community server process if running
    {
        // Try graceful shutdown via IPC first (same as graceful_shutdown)
        let socket_path = crate::ipc_client::default_socket_path();
        if socket_path.exists() {
            if let Err(e) = crate::ipc_client::shutdown_server_blocking(&socket_path) {
                tracing::debug!(error = %e, "IPC shutdown failed on logout — will kill process");
            }
        }

        let mut proc = state.server_process.lock();
        if let Some(ref mut child) = *proc {
            let pid = child.id();
            // Give the server a moment to exit gracefully after IPC shutdown
            if !matches!(child.try_wait(), Ok(Some(_))) {
                tracing::info!(pid, "killing rekindle-server");
                let _ = child.kill();
                let _ = child.wait();
            }
        }
        *proc = None;
    }

    // NOTE: Do NOT reset network_ready_tx here. The Veilid node is still alive
    // and attached — the network IS ready. Resetting to false would cause the
    // next login's spawn_dht_publish() to time out waiting for a readiness signal
    // that never arrives (no new Attachment event fires when the node is already attached).

    tracing::info!("logout cleanup complete — node still running");
}

/// Shutdown the Veilid node (called only on app exit).
///
/// Follows the veilid-server shutdown ordering:
/// 1. Signal dispatch loop shutdown
/// 2. Close remaining DHT records
/// 3. Release private route and clear managers
/// 4. `api.shutdown().await`
pub async fn shutdown_app(state: &AppState) {
    // 1. Abort all remaining background tasks
    {
        let mut handles = state.background_handles.lock();
        for handle in handles.drain(..) {
            handle.abort();
        }
    }

    // 2. Close ALL open DHT records tracked during this session
    {
        let rc_and_keys = {
            let node = state.node.read();
            let rc = node.as_ref().map(|nh| nh.routing_context.clone());
            let keys: Vec<String> = {
                let dht_mgr = state.dht_manager.read();
                dht_mgr
                    .as_ref()
                    .map(|mgr| mgr.open_records.iter().cloned().collect())
                    .unwrap_or_default()
            };
            rc.map(|rc| (rc, keys))
        };
        if let Some((rc, keys)) = rc_and_keys {
            tracing::debug!(count = keys.len(), "closing open DHT records for app exit");
            for key_str in &keys {
                if let Ok(record_key) = key_str.parse::<veilid_core::RecordKey>() {
                    if let Err(e) = rc.close_dht_record(record_key).await {
                        tracing::trace!(key = %key_str, error = %e, "close DHT record on app exit");
                    }
                }
            }
        }
    }

    // 3. Release private route and clear managers
    {
        let mut rm = state.routing_manager.write();
        if let Some(ref mut handle) = *rm {
            if let Err(e) = handle.manager.release_private_route() {
                tracing::warn!(error = %e, "failed to release private route during app exit");
            }
        }
        *rm = None;
    }
    *state.dht_manager.write() = None;

    // 4. Shutdown the Veilid API
    let api = {
        let mut node = state.node.write();
        node.take().map(|nh| nh.api)
    };
    if let Some(api) = api {
        api.shutdown().await;
    }

    tracing::info!("veilid node shut down");
}
