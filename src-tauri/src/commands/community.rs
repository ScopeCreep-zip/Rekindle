use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};

use crate::channels::ChatEvent;
use crate::commands::chat::Message;
use crate::db::{self, DbPool};
use crate::db_helpers::{db_call, db_fire};
use crate::keystore::KeystoreHandle;
use crate::services;
use crate::state::{ChannelType, SharedState};
use crate::state_helpers;

/// A community member for the frontend.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberDto {
    pub pseudonym_key: String,
    pub display_name: String,
    pub role_ids: Vec<u32>,
    pub display_role: String,
    pub status: String,
    pub timeout_until: Option<u64>,
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
pub async fn get_communities(state: State<'_, SharedState>) -> Result<Vec<CommunityInfo>, String> {
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

/// Role DTO for frontend consumption (re-exports the channel's `RoleDto`).
pub use crate::channels::community_channel::RoleDto as CommunityRoleDto;

/// Full community detail with channels for the frontend.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityDetail {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub channels: Vec<ChannelInfoDto>,
    pub my_role: Option<String>,
    pub my_role_ids: Vec<u32>,
    pub roles: Vec<CommunityRoleDto>,
    pub my_pseudonym_key: Option<String>,
    pub mek_generation: u64,
    pub is_hosted: bool,
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
            my_role_ids: c.my_role_ids.clone(),
            roles: c.roles.iter().map(CommunityRoleDto::from).collect(),
            my_pseudonym_key: c.my_pseudonym_key.clone(),
            mek_generation: c.mek_generation,
            is_hosted: c.is_hosted,
        })
        .collect();
    Ok(list)
}

/// Create a new community and store it in `AppState` + `SQLite`.
#[tauri::command]
pub async fn create_community(
    app: tauri::AppHandle,
    name: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    keystore_handle: State<'_, KeystoreHandle>,
) -> Result<String, String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    let community_id = services::community_service::create_community(state.inner(), &name).await?;

    // Persist MEK to Stronghold for login restoration
    {
        let mek_cache = state.mek_cache.lock();
        if let Some(mek) = mek_cache.get(&community_id) {
            let ks = keystore_handle.lock();
            if let Some(ref keystore) = *ks {
                crate::keystore::persist_mek(keystore, &community_id, mek);
            }
        }
    }

    // Read back the community to get default channel info
    let community = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .cloned()
            .ok_or("community not found after creation")?
    };

    // Read creator identity outside db_call (parking_lot guard is !Send)
    let creator_key = owner_key.clone();
    let creator_name = state_helpers::identity_display_name(state.inner());

    // Get pseudonym key for this community
    let my_pseudonym_key = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .and_then(|c| c.my_pseudonym_key.clone())
    };

    let now = db::timestamp_now();
    let community_id_clone = community_id.clone();
    let name_clone = name.clone();
    let dht_record_key = community.dht_record_key.clone();
    let dht_owner_keypair = community.dht_owner_keypair.clone();
    let pseudonym_key = my_pseudonym_key
        .clone()
        .unwrap_or_else(|| creator_key.clone());
    let roles_to_persist = community.roles.clone();
    let mek_gen = community.mek_generation.cast_signed();
    let ok = owner_key;
    db_call(pool.inner(), move |conn| {
        // Owner gets all default role IDs: @everyone(0), members(1), moderator(2), admin(3), owner(4)
        let owner_role_ids = serde_json::to_string(&[0u32, 1, 2, 3, 4]).unwrap_or_default();
        conn.execute(
            "INSERT INTO communities (owner_key, id, name, my_role, my_role_ids, joined_at, dht_record_key, dht_owner_keypair, my_pseudonym_key, is_hosted, mek_generation) \
             VALUES (?, ?, ?, 'owner', ?, ?, ?, ?, ?, 1, ?)",
            rusqlite::params![ok, community_id_clone, name_clone, owner_role_ids, now, dht_record_key, dht_owner_keypair, pseudonym_key, mek_gen],
        )?;

        // Insert the creator as the first member (using pseudonym)
        conn.execute(
            "INSERT INTO community_members (owner_key, community_id, pseudonym_key, display_name, role_ids, joined_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
            rusqlite::params![ok, community_id_clone, pseudonym_key, creator_name, owner_role_ids, now],
        )?;

        // Insert default channels
        for channel in &community.channels {
            let ch_type = match channel.channel_type {
                ChannelType::Text => "text",
                ChannelType::Voice => "voice",
            };
            conn.execute(
                "INSERT INTO channels (owner_key, id, community_id, name, channel_type) VALUES (?, ?, ?, ?, ?)",
                rusqlite::params![ok, channel.id, community_id_clone, channel.name, ch_type],
            )?;
        }

        // Persist default roles
        for r in &roles_to_persist {
            conn.execute(
                "INSERT INTO community_roles (owner_key, community_id, role_id, name, color, permissions, position, hoist, mentionable) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    ok, community_id_clone, r.id, r.name, r.color,
                    r.permissions.cast_signed(), r.position, i32::from(r.hoist), i32::from(r.mentionable),
                ],
            )?;
        }

        Ok(())
    })
    .await?;

    ensure_community_hosted(&app, state.inner(), &community_id).await;

    Ok(community_id)
}

/// Join an existing community by ID.
#[tauri::command]
#[allow(clippy::too_many_lines)]
pub async fn join_community(
    community_id: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    keystore_handle: State<'_, KeystoreHandle>,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    services::community_service::join_community(state.inner(), &community_id).await?;

    let (name, dht_record_key) = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .map(|c| (c.name.clone(), c.dht_record_key.clone()))
            .unwrap_or_default()
    };

    // Read joiner identity outside db_call (parking_lot guard is !Send)
    let joiner_name = state_helpers::identity_display_name(state.inner());

    // Get pseudonym key, server_route_blob, mek_generation, and channels from the community state
    let (my_pseudonym_key, server_route_blob, mek_generation, channels) = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .map(|c| {
                (
                    c.my_pseudonym_key.clone(),
                    c.server_route_blob.clone(),
                    c.mek_generation,
                    c.channels.clone(),
                )
            })
            .unwrap_or_default()
    };
    let pseudonym_key = my_pseudonym_key.unwrap_or_else(|| owner_key.clone());

    // Persist MEK to Stronghold for login restoration
    {
        let mek_cache = state.mek_cache.lock();
        if let Some(mek) = mek_cache.get(&community_id) {
            let ks = keystore_handle.lock();
            if let Some(ref keystore) = *ks {
                crate::keystore::persist_mek(keystore, &community_id, mek);
            }
        }
    }

    // Get role_ids and roles from community state (set by join RPC response)
    let (my_role_ids, roles_to_persist) = {
        let communities = state.communities.read();
        match communities.get(&community_id) {
            Some(c) => (c.my_role_ids.clone(), c.roles.clone()),
            None => (vec![0, 1], Vec::new()),
        }
    };
    let role_ids_json = serde_json::to_string(&my_role_ids).unwrap_or_else(|_| "[0,1]".to_string());

    let now = db::timestamp_now();
    let community_id_clone = community_id.clone();
    let ok = owner_key;
    let pk = pseudonym_key.clone();
    let srb = server_route_blob.clone();
    let mg = mek_generation.cast_signed();
    let rij = role_ids_json.clone();
    db_call(pool.inner(), move |conn| {
        conn.execute(
            "INSERT OR IGNORE INTO communities (owner_key, id, name, my_role, my_role_ids, joined_at, dht_record_key, my_pseudonym_key, server_route_blob, mek_generation) \
             VALUES (?, ?, ?, 'member', ?, ?, ?, ?, ?, ?)",
            rusqlite::params![ok, community_id_clone, name, rij, now, dht_record_key, pk, srb, mg],
        )?;

        // Add ourselves to the community_members table (using pseudonym)
        conn.execute(
            "INSERT OR IGNORE INTO community_members (owner_key, community_id, pseudonym_key, display_name, role_ids, joined_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
            rusqlite::params![ok, community_id_clone, pk, joiner_name, rij, now],
        )?;

        // Persist channels to SQLite so they survive re-login
        for channel in &channels {
            let ch_type = match channel.channel_type {
                crate::state::ChannelType::Text => "text",
                crate::state::ChannelType::Voice => "voice",
            };
            conn.execute(
                "INSERT OR IGNORE INTO channels (owner_key, id, community_id, name, channel_type) VALUES (?, ?, ?, ?, ?)",
                rusqlite::params![ok, channel.id, community_id_clone, channel.name, ch_type],
            )?;
        }

        // Persist roles from server
        for r in &roles_to_persist {
            conn.execute(
                "INSERT OR IGNORE INTO community_roles (owner_key, community_id, role_id, name, color, permissions, position, hoist, mentionable) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    ok, community_id_clone, r.id, r.name, r.color,
                    r.permissions.cast_signed(), r.position, i32::from(r.hoist), i32::from(r.mentionable),
                ],
            )?;
        }

        Ok(())
    })
    .await?;

    Ok(())
}

/// Create a new channel in a community.
///
/// For hosted communities, sends a `CommunityRequest::CreateChannel` to the
/// server. For local-only communities, creates the channel locally + DHT.
#[tauri::command]
pub async fn create_channel(
    community_id: String,
    name: String,
    channel_type: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<String, String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;

    // If this is a hosted community, send CreateChannel RPC to the server.
    // send_community_rpc will on-demand fetch the route blob if missing.
    let has_community = {
        let communities = state.communities.read();
        communities.contains_key(&community_id)
    };

    if has_community {
        let response = send_community_rpc(
            state.inner(),
            pool.inner(),
            &community_id,
            rekindle_protocol::messaging::CommunityRequest::CreateChannel {
                name: name.clone(),
                channel_type: channel_type.clone(),
            },
        )
        .await;

        match response {
            Ok(rekindle_protocol::messaging::CommunityResponse::ChannelCreated { channel_id }) => {
                // Server created the channel — add it to local state too
                let ch_type = match channel_type.as_str() {
                    "voice" => ChannelType::Voice,
                    _ => ChannelType::Text,
                };
                {
                    let mut communities = state.communities.write();
                    if let Some(community) = communities.get_mut(&community_id) {
                        community.channels.push(crate::state::ChannelInfo {
                            id: channel_id.clone(),
                            name: name.clone(),
                            channel_type: ch_type,
                            unread_count: 0,
                        });
                    }
                }

                // Persist to local SQLite
                let comm_id = community_id.clone();
                let chan_id = channel_id.clone();
                let n = name.clone();
                let ct = channel_type.clone();
                db_call(pool.inner(), move |conn| {
                    conn.execute(
                        "INSERT INTO channels (owner_key, id, community_id, name, channel_type) VALUES (?, ?, ?, ?, ?)",
                        rusqlite::params![owner_key, chan_id, comm_id, n, ct],
                    )?;
                    Ok(())
                })
                .await?;

                return Ok(channel_id);
            }
            Ok(rekindle_protocol::messaging::CommunityResponse::Error { message, .. }) => {
                return Err(format!("server rejected channel creation: {message}"));
            }
            Ok(other) => {
                return Err(format!(
                    "unexpected server response for CreateChannel: {other:?}"
                ));
            }
            Err(e) => {
                tracing::warn!(
                    community = %community_id, error = %e,
                    "server unreachable for CreateChannel — falling back to local-only"
                );
                // Fall through to local-only creation below
            }
        }
    }

    // Local-only channel creation (no server route, or server was unreachable)
    let channel_id = services::community_service::create_channel(
        state.inner(),
        &community_id,
        &name,
        &channel_type,
    )
    .await?;

    let channel_id_clone = channel_id.clone();
    let community_id_clone = community_id.clone();
    let name_clone = name.clone();
    let channel_type_clone = channel_type.clone();
    db_call(pool.inner(), move |conn| {
        conn.execute(
            "INSERT INTO channels (owner_key, id, community_id, name, channel_type) VALUES (?, ?, ?, ?, ?)",
            rusqlite::params![owner_key, channel_id_clone, community_id_clone, name_clone, channel_type_clone],
        )?;
        Ok(())
    })
    .await?;

    Ok(channel_id)
}

/// Send a message in a community channel.
///
/// Encrypts the message body with the community's MEK, then sends a
/// `CommunityRequest::SendMessage` to the community server via `app_call`.
/// Falls back to local-only storage if the server is unreachable.
#[tauri::command]
pub async fn send_channel_message(
    channel_id: String,
    body: String,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;

    let timestamp = db::timestamp_now();

    // --- Step 1: Find the community and get MEK + server route + pseudonym ---
    let (community_id, mek_generation, server_route_blob) = {
        let communities = state.communities.read();
        let community = communities
            .values()
            .find(|c| c.channels.iter().any(|ch| ch.id == channel_id))
            .ok_or("channel not found in any community")?;
        (
            community.id.clone(),
            community.mek_generation,
            community.server_route_blob.clone(),
        )
    };

    // Use pseudonym key as sender for channel messages (matches what the
    // server broadcasts to other members, keeping sender IDs consistent)
    let sender_key = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .and_then(|c| c.my_pseudonym_key.clone())
            .unwrap_or_else(|| owner_key.clone())
    };

    // --- Step 2: Encrypt with MEK ---
    let ciphertext = {
        let mek_cache = state.mek_cache.lock();
        let mek = mek_cache.get(&community_id).ok_or_else(|| {
            "MEK not available — rejoin the community or wait for MEK delivery".to_string()
        })?;
        mek.encrypt(body.as_bytes())
            .map_err(|e| format!("MEK encryption failed: {e}"))?
    };

    // --- Step 3: Store plaintext in local SQLite FIRST (persist before send) ---
    let pool_for_queue = pool.inner().clone();
    let channel_id_clone = channel_id.clone();
    let sender_key_clone = sender_key.clone();
    let body_clone = body.clone();
    let ok = owner_key;
    db_call(pool.inner(), move |conn| {
        conn.execute(
            "INSERT INTO messages (owner_key, conversation_id, conversation_type, sender_key, body, timestamp, is_read, mek_generation) \
             VALUES (?, ?, 'channel', ?, ?, ?, 1, ?)",
            rusqlite::params![ok, channel_id_clone, sender_key_clone, body_clone, timestamp, mek_generation.cast_signed()],
        )?;
        Ok(())
    })
    .await?;

    // --- Step 4: Send to community server (best-effort — message already persisted) ---
    if let Some(route_blob) = server_route_blob {
        if let Err(e) = send_encrypted_to_server(
            &state,
            &channel_id,
            &community_id,
            ciphertext.clone(),
            mek_generation,
            timestamp,
            route_blob,
        )
        .await
        {
            tracing::warn!(error = %e, "server delivery failed — queuing for retry");
            queue_pending_channel_message(
                &state,
                &pool_for_queue,
                &community_id,
                &channel_id,
                &ciphertext,
                mek_generation,
                timestamp,
            );
        }
    } else {
        tracing::warn!("no server route — message stored locally, queuing for retry");
        queue_pending_channel_message(
            &state,
            &pool_for_queue,
            &community_id,
            &channel_id,
            &ciphertext,
            mek_generation,
            timestamp,
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

/// Pending channel message queued for retry delivery to the community server.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PendingChannelMessage {
    pub community_id: String,
    pub channel_id: String,
    pub ciphertext: Vec<u8>,
    pub mek_generation: u64,
    pub timestamp: i64,
}

/// Queue a failed channel message for retry via `pending_messages` table.
///
/// Serializes as JSON into the `body` column. Uses `community_id` as `recipient_key`
/// so `sync_service` can distinguish channel retries from DM retries.
fn queue_pending_channel_message(
    state: &SharedState,
    pool: &DbPool,
    community_id: &str,
    channel_id: &str,
    ciphertext: &[u8],
    mek_generation: u64,
    timestamp: i64,
) {
    let pending = PendingChannelMessage {
        community_id: community_id.to_string(),
        channel_id: channel_id.to_string(),
        ciphertext: ciphertext.to_vec(),
        mek_generation,
        timestamp,
    };
    let body = match serde_json::to_string(&pending) {
        Ok(b) => b,
        Err(e) => {
            tracing::error!(error = %e, "failed to serialize pending channel message");
            return;
        }
    };

    let owner_key = state
        .identity
        .read()
        .as_ref()
        .map(|id| id.public_key.clone())
        .unwrap_or_default();
    let recipient = community_id.to_string();
    let now = crate::db::timestamp_now();
    db_fire(pool, "queue pending channel message", move |conn| {
        conn.execute(
            "INSERT INTO pending_messages (owner_key, recipient_key, body, created_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![owner_key, recipient, body, now],
        )?;
        Ok(())
    });
}

fn rand_nonce() -> Vec<u8> {
    use rand::RngCore;
    let mut nonce = vec![0u8; 24];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    nonce
}

/// Send an encrypted channel message to the community server via Veilid `app_call`.
///
/// Builds a signed envelope with the user's pseudonym key and sends it to the
/// server's private route. Returns `Err` on transport or route import failures
/// so the caller can queue for retry. The message is already stored locally
/// before this is called.
pub(crate) async fn send_encrypted_to_server(
    state: &SharedState,
    channel_id: &str,
    community_id: &str,
    ciphertext: Vec<u8>,
    mek_generation: u64,
    timestamp: i64,
    route_blob: Vec<u8>,
) -> Result<(), String> {
    let Some((api, rc)) = state_helpers::api_and_routing_context(state) else {
        return Ok(());
    };

    let request = rekindle_protocol::messaging::CommunityRequest::SendMessage {
        channel_id: channel_id.to_string(),
        ciphertext,
        mek_generation,
    };
    let request_bytes =
        serde_json::to_vec(&request).map_err(|e| format!("failed to serialize request: {e}"))?;

    let identity_secret = {
        let secret = state.identity_secret.lock();
        *secret
    };
    let Some(secret) = identity_secret else {
        return Ok(());
    };

    let pseudonym_key =
        rekindle_crypto::group::pseudonym::derive_community_pseudonym(&secret, community_id);
    let envelope = rekindle_protocol::messaging::sender::build_envelope(
        &pseudonym_key,
        timestamp.cast_unsigned(),
        rand_nonce(),
        request_bytes,
    );

    // Use the DHTManager's route cache to avoid leaking RouteId objects.
    // Each call to import_remote_private_route without caching creates a new
    // RouteId that Veilid tracks internally — the cache reuses them.
    let route_id = {
        let mut dht_mgr = state.dht_manager.write();
        match dht_mgr.as_mut() {
            Some(mgr) => mgr
                .manager
                .get_or_import_route(&api, &route_blob)
                .map_err(|e| format!("failed to import server route: {e}"))?,
            None => api
                .import_remote_private_route(route_blob.clone())
                .map_err(|e| format!("failed to import server route: {e}"))?,
        }
    };

    let result = rekindle_protocol::messaging::sender::send_call(&rc, route_id, &envelope).await;
    match result {
        Ok(response_bytes) => {
            check_server_response(channel_id, &response_bytes)?;
        }
        Err(e) => {
            // Invalidate the stale route from DHTManager cache so the next
            // attempt (e.g. from the pending message retry queue) forces a
            // fresh import instead of reusing the dead RouteId.
            {
                let mut dht_mgr = state.dht_manager.write();
                if let Some(mgr) = dht_mgr.as_mut() {
                    mgr.manager.invalidate_route_blob(&route_blob);
                }
            }
            // Also clear the in-memory route blob so next send fetches from DHT.
            {
                let mut communities = state.communities.write();
                if let Some(c) = communities.get_mut(community_id) {
                    c.server_route_blob = None;
                }
            }
            return Err(format!("failed to send channel message to server: {e}"));
        }
    }

    Ok(())
}

/// Check the server's response to a channel message send attempt.
fn check_server_response(channel_id: &str, response_bytes: &[u8]) -> Result<(), String> {
    match serde_json::from_slice::<rekindle_protocol::messaging::CommunityResponse>(response_bytes)
    {
        Ok(rekindle_protocol::messaging::CommunityResponse::Ok) => {
            tracing::debug!(channel = %channel_id, "channel message sent to server");
            Ok(())
        }
        Ok(rekindle_protocol::messaging::CommunityResponse::Error { message, .. }) => {
            Err(format!("server rejected channel message: {message}"))
        }
        _ => {
            tracing::debug!(channel = %channel_id, "unexpected server response");
            Ok(())
        }
    }
}

/// Send a community RPC request to the server.
///
/// For **hosted** communities (where `is_hosted = true`), routes the request
/// through the local Unix socket IPC — bypassing Veilid entirely. This avoids
/// the unreliable same-machine P2P route discovery that causes timeouts.
///
/// For **remote** communities, signs the request with the user's pseudonym key,
/// wraps it in a `MessageEnvelope`, and sends it via Veilid `app_call`.
async fn send_community_rpc(
    state: &SharedState,
    pool: &DbPool,
    community_id: &str,
    request: rekindle_protocol::messaging::CommunityRequest,
) -> Result<rekindle_protocol::messaging::CommunityResponse, String> {
    let (exists, is_hosted) = {
        let communities = state.communities.read();
        match communities.get(community_id) {
            Some(c) => (true, c.is_hosted),
            None => (false, false),
        }
    };
    if !exists {
        return Err("community not found".into());
    }

    if is_hosted {
        return send_community_rpc_ipc(state, community_id, &request).await;
    }

    send_community_rpc_veilid(state, pool, community_id, request).await
}

/// IPC fast path: send the RPC through the local Unix socket to the server process.
async fn send_community_rpc_ipc(
    state: &SharedState,
    community_id: &str,
    request: &rekindle_protocol::messaging::CommunityRequest,
) -> Result<rekindle_protocol::messaging::CommunityResponse, String> {
    let pseudonym_key = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .and_then(|c| c.my_pseudonym_key.clone())
            .ok_or_else(|| "no pseudonym key for this community".to_string())?
    };

    let request_json =
        serde_json::to_string(request).map_err(|e| format!("failed to serialize request: {e}"))?;

    let socket_path = crate::ipc_client::default_socket_path();
    let cid = community_id.to_string();
    let response_json = tokio::task::spawn_blocking(move || {
        crate::ipc_client::community_rpc_blocking(&socket_path, &cid, &pseudonym_key, &request_json)
    })
    .await
    .map_err(|e| format!("IPC task panicked: {e}"))??;

    serde_json::from_str(&response_json).map_err(|e| format!("invalid IPC response: {e}"))
}

/// Veilid path: sign + envelope + `app_call` for remote communities.
async fn send_community_rpc_veilid(
    state: &SharedState,
    pool: &DbPool,
    community_id: &str,
    request: rekindle_protocol::messaging::CommunityRequest,
) -> Result<rekindle_protocol::messaging::CommunityResponse, String> {
    let server_route_blob = resolve_server_route_blob(state, pool, community_id).await?;

    let (api, rc) = state_helpers::api_and_routing_context(state)
        .ok_or_else(|| "Veilid network not attached".to_string())?;

    let signing_key = {
        let secret = state.identity_secret.lock();
        let s = (*secret).ok_or_else(|| "identity not unlocked".to_string())?;
        rekindle_crypto::group::pseudonym::derive_community_pseudonym(&s, community_id)
    };

    let request_bytes =
        serde_json::to_vec(&request).map_err(|e| format!("failed to serialize request: {e}"))?;
    let timestamp = crate::db::timestamp_now().cast_unsigned();
    let envelope = rekindle_protocol::messaging::sender::build_envelope(
        &signing_key,
        timestamp,
        rand_nonce(),
        request_bytes,
    );

    let route_id = {
        let mut dht_mgr = state.dht_manager.write();
        match dht_mgr.as_mut() {
            Some(mgr) => mgr
                .manager
                .get_or_import_route(&api, &server_route_blob)
                .map_err(|e| format!("RPC call failed: {e}"))?,
            None => api
                .import_remote_private_route(server_route_blob)
                .map_err(|e| format!("RPC call failed: {e}"))?,
        }
    };

    let result = rekindle_protocol::messaging::sender::send_call(&rc, route_id, &envelope).await;

    match result {
        Ok(response_bytes) => parse_community_response(&response_bytes),
        Err(e) => {
            retry_rpc_with_fresh_route(state, pool, community_id, &rc, &api, &envelope, &e).await
        }
    }
}

/// Resolve the server route blob: try in-memory cache, then on-demand DHT fetch.
async fn resolve_server_route_blob(
    state: &SharedState,
    pool: &DbPool,
    community_id: &str,
) -> Result<Vec<u8>, String> {
    let cached = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .and_then(|c| c.server_route_blob.clone())
    };

    if let Some(blob) = cached {
        return Ok(blob);
    }

    tracing::info!(community = %community_id, "server route blob missing — on-demand DHT fetch");
    let blob = fetch_server_route_blob_quick(state, community_id)
        .await
        .ok_or_else(|| {
            "community server route not available (server may still be starting)".to_string()
        })?;

    {
        let mut communities = state.communities.write();
        if let Some(c) = communities.get_mut(community_id) {
            c.server_route_blob = Some(blob.clone());
        }
    }
    persist_route_blob_to_db(pool, state, community_id, &blob);

    Ok(blob)
}

/// Parse a community RPC response.
fn parse_community_response(
    response_bytes: &[u8],
) -> Result<rekindle_protocol::messaging::CommunityResponse, String> {
    serde_json::from_slice(response_bytes).map_err(|e| format!("invalid response from server: {e}"))
}

/// On RPC failure, clear the stale route, fetch a fresh one from DHT, and retry once.
async fn retry_rpc_with_fresh_route(
    state: &SharedState,
    pool: &DbPool,
    community_id: &str,
    rc: &veilid_core::RoutingContext,
    api: &veilid_core::VeilidAPI,
    envelope: &rekindle_protocol::messaging::envelope::MessageEnvelope,
    original_error: &rekindle_protocol::error::ProtocolError,
) -> Result<rekindle_protocol::messaging::CommunityResponse, String> {
    tracing::warn!(
        error = %original_error,
        community = %community_id,
        "community RPC failed — retrying with fresh route from DHT"
    );

    // Invalidate the stale route blob from DHTManager cache so the retry
    // doesn't return the same dead RouteId (the fresh DHT fetch may return
    // identical blob bytes if the server hasn't refreshed yet).
    let stale_blob = {
        let mut communities = state.communities.write();
        let old = communities
            .get(community_id)
            .and_then(|c| c.server_route_blob.clone());
        if let Some(c) = communities.get_mut(community_id) {
            c.server_route_blob = None;
        }
        old
    };
    if let Some(ref blob) = stale_blob {
        let mut dht_mgr = state.dht_manager.write();
        if let Some(mgr) = dht_mgr.as_mut() {
            mgr.manager.invalidate_route_blob(blob);
        }
    }

    let fresh_blob = fetch_server_route_blob_quick(state, community_id)
        .await
        .ok_or_else(|| format!("RPC failed (no fresh route on retry): {original_error}"))?;

    {
        let mut communities = state.communities.write();
        if let Some(c) = communities.get_mut(community_id) {
            c.server_route_blob = Some(fresh_blob.clone());
        }
    }
    persist_route_blob_to_db(pool, state, community_id, &fresh_blob);

    let fresh_route_id = {
        let mut dht_mgr = state.dht_manager.write();
        match dht_mgr.as_mut() {
            Some(mgr) => mgr
                .manager
                .get_or_import_route(api, &fresh_blob)
                .map_err(|e| format!("RPC retry failed: {e}"))?,
            None => api
                .import_remote_private_route(fresh_blob)
                .map_err(|e| format!("RPC retry failed: {e}"))?,
        }
    };

    rekindle_protocol::messaging::sender::send_call(rc, fresh_route_id, envelope)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, community = %community_id, "community RPC retry also failed");
            format!("RPC call failed after retry: {e}")
        })
        .and_then(|bytes| parse_community_response(&bytes))
}

/// Quick on-demand fetch of the server route blob from DHT (3 retries, 2s apart).
///
/// Used by `send_community_rpc` to self-heal when the blob is missing from
/// in-memory state. Faster than the full `fetch_server_route_blob` (10 retries,
/// 3s apart) since the server is likely already running.
async fn fetch_server_route_blob_quick(state: &SharedState, community_id: &str) -> Option<Vec<u8>> {
    let dht_record_key = {
        let communities = state.communities.read();
        communities.get(community_id)?.dht_record_key.clone()?
    };

    let routing_context = {
        let node = state.node.read();
        let nh = node.as_ref()?;
        nh.routing_context.clone()
    };

    for attempt in 0..3u32 {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }

        let mgr = rekindle_protocol::dht::DHTManager::new(routing_context.clone());
        if mgr.open_record(&dht_record_key).await.is_err() {
            continue;
        }

        let blob = mgr
            .get_value(
                &dht_record_key,
                rekindle_protocol::dht::community::SUBKEY_SERVER_ROUTE,
            )
            .await
            .ok()
            .flatten();

        let _ = mgr.close_record(&dht_record_key).await;

        if blob.is_some() {
            tracing::info!(
                community = %community_id,
                attempt,
                "on-demand DHT fetch found server route blob"
            );
            return blob;
        }
    }

    tracing::warn!(
        community = %community_id,
        "on-demand DHT fetch failed after 3 attempts"
    );
    None
}

/// Persist a server route blob to `SQLite` (without requiring `AppHandle`).
fn persist_route_blob_to_db(
    pool: &DbPool,
    state: &SharedState,
    community_id: &str,
    route_blob: &[u8],
) {
    let owner_key = state
        .identity
        .read()
        .as_ref()
        .map(|id| id.public_key.clone())
        .unwrap_or_default();
    let cid = community_id.to_string();
    let blob = route_blob.to_vec();
    db_fire(pool, "persist server_route_blob", move |conn| {
        conn.execute(
            "UPDATE communities SET server_route_blob = ?1 WHERE owner_key = ?2 AND id = ?3",
            rusqlite::params![blob, owner_key, cid],
        )?;
        Ok(())
    });
}

/// Leave a community and clean up local state.
///
/// Sends `CommunityRequest::Leave` to the server (which triggers MEK rotation
/// for remaining members), then cleans up local state and `SQLite`.
#[tauri::command]
pub async fn leave_community(
    community_id: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    keystore_handle: State<'_, KeystoreHandle>,
) -> Result<(), String> {
    // Send Leave RPC to the community server before cleaning up locally
    // Best-effort: ignore errors since we're leaving anyway
    let _ = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::Leave,
    )
    .await;

    // Remove MEK from cache
    state.mek_cache.lock().remove(&community_id);

    // Remove MEK from Stronghold
    {
        use rekindle_crypto::keychain::{mek_key_name, VAULT_COMMUNITIES};
        use rekindle_crypto::Keychain as _;

        let ks = keystore_handle.lock();
        if let Some(ref keystore) = *ks {
            let key_name = mek_key_name(&community_id);
            if let Err(e) = keystore.delete_key(VAULT_COMMUNITIES, &key_name) {
                tracing::warn!(error = %e, "failed to remove MEK from Stronghold");
            }
        }
    }

    // Remove cached server route
    state.community_routes.write().remove(&community_id);

    // Remove from local state
    state.communities.write().remove(&community_id);

    // Remove from SQLite (CASCADE on communities handles channels)
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    let community_id_clone = community_id.clone();
    db_call(pool.inner(), move |conn| {
        conn.execute(
            "DELETE FROM communities WHERE owner_key = ? AND id = ?",
            rusqlite::params![owner_key, community_id_clone],
        )?;
        Ok(())
    })
    .await?;

    tracing::info!(community = %community_id, "left community");
    Ok(())
}

/// Get message history for a community channel.
///
/// First queries local `SQLite`. If local DB has no messages for the channel,
/// fetches history from the community server via `CommunityRequest::GetMessages`,
/// decrypts the ciphertexts with the cached MEK, and stores them locally.
#[tauri::command]
pub async fn get_channel_messages(
    channel_id: String,
    limit: u32,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<Vec<Message>, String> {
    let our_key = state_helpers::current_owner_key(state.inner()).unwrap_or_default();

    // Our pseudonym key for this channel's community (for is_own detection)
    let (community_id, my_pseudonym_key) = {
        let communities = state.communities.read();
        let community = communities
            .values()
            .find(|c| c.channels.iter().any(|ch| ch.id == channel_id));
        match community {
            Some(c) => (
                Some(c.id.clone()),
                c.my_pseudonym_key.clone().unwrap_or_default(),
            ),
            None => (None, String::new()),
        }
    };

    // --- Step 1: Query local SQLite (returns immediately) ---
    let channel_id_clone = channel_id.clone();
    let ok = our_key.clone();
    let mpk = my_pseudonym_key.clone();
    let mut messages = db_call(pool.inner(), move |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, sender_key, body, timestamp FROM messages \
                 WHERE owner_key = ? AND conversation_id = ? AND conversation_type = 'channel' \
                 ORDER BY timestamp DESC LIMIT ?",
        )?;

        let rows = stmt.query_map(rusqlite::params![ok, channel_id_clone, limit], |row| {
            let sender = db::get_str(row, "sender_key");
            // is_own: match against either our owner_key or pseudonym_key
            let is_own = sender == ok || sender == mpk;
            Ok(Message {
                id: db::get_i64(row, "id"),
                sender_id: sender,
                body: db::get_str(row, "body"),
                timestamp: db::get_i64(row, "timestamp"),
                is_own,
            })
        })?;

        let mut messages = Vec::new();
        for row in rows {
            messages.push(row?);
        }
        Ok(messages)
    })
    .await?;

    // Reverse so messages are in chronological order (query was DESC for most-recent)
    messages.reverse();

    tracing::debug!(
        owner_key = %our_key,
        channel_id = %channel_id,
        local_count = messages.len(),
        "loaded channel messages from local DB"
    );

    // --- Step 2: Background server fetch for missed messages ---
    // Spawn a background task so the frontend gets local messages immediately.
    if let Some(cid) = community_id {
        let state = state.inner().clone();
        let pool = pool.inner().clone();
        let channel_id = channel_id.clone();
        let our_key = our_key.clone();
        let my_pseudonym_key = my_pseudonym_key.clone();
        tokio::spawn(async move {
            let server_messages = fetch_channel_history_from_server(
                &state,
                &pool,
                &cid,
                &channel_id,
                &our_key,
                &my_pseudonym_key,
                limit,
            )
            .await;
            if !server_messages.is_empty() {
                tracing::debug!(
                    channel_id = %channel_id,
                    server_count = server_messages.len(),
                    "background server fetch returned messages"
                );
                let _ = app.emit(
                    "chat-event",
                    ChatEvent::ChannelHistoryLoaded {
                        channel_id,
                        messages: server_messages,
                    },
                );
            }
        });
    }

    Ok(messages)
}

/// Fetch message history from the community server, decrypt, and store locally.
async fn fetch_channel_history_from_server(
    state: &SharedState,
    pool: &DbPool,
    community_id: &str,
    channel_id: &str,
    owner_key: &str,
    my_pseudonym_key: &str,
    limit: u32,
) -> Vec<Message> {
    let response = send_community_rpc(
        state,
        pool,
        community_id,
        rekindle_protocol::messaging::CommunityRequest::GetMessages {
            channel_id: channel_id.to_string(),
            before_timestamp: None,
            limit,
        },
    )
    .await;

    let Ok(rekindle_protocol::messaging::CommunityResponse::Messages {
        messages: server_messages,
    }) = response
    else {
        return Vec::new();
    };

    if server_messages.is_empty() {
        return Vec::new();
    }

    // Decrypt with cached MEK — scope the guard so it's dropped before any .await
    let decrypted: Vec<(String, String, i64, i64)> = {
        let mek_cache = state.mek_cache.lock();
        let Some(mek) = mek_cache.get(community_id) else {
            tracing::warn!(community = %community_id, "no MEK to decrypt server history");
            return Vec::new();
        };

        let mut result = Vec::new();
        for msg in &server_messages {
            if msg.mek_generation != mek.generation() {
                tracing::debug!(
                    have = mek.generation(),
                    need = msg.mek_generation,
                    "skipping message with different MEK generation"
                );
                continue;
            }
            match mek.decrypt(&msg.ciphertext) {
                Ok(plaintext) => {
                    let body = String::from_utf8(plaintext).unwrap_or_default();
                    result.push((
                        msg.sender_pseudonym.clone(),
                        body,
                        msg.timestamp.cast_signed(),
                        msg.mek_generation.cast_signed(),
                    ));
                }
                Err(e) => {
                    tracing::debug!(error = %e, "failed to decrypt historical message");
                }
            }
        }
        result
    };

    // Store decrypted messages in local SQLite (fire-and-forget)
    let ok = owner_key.to_string();
    let cid = channel_id.to_string();
    let mpk = my_pseudonym_key.to_string();
    let decrypted_clone = decrypted.clone();
    db_fire(pool, "store decrypted channel history", move |conn| {
        for (sender, body, ts, mg) in &decrypted_clone {
            let _ = conn.execute(
                "INSERT OR IGNORE INTO messages (owner_key, conversation_id, conversation_type, sender_key, body, timestamp, is_read, mek_generation) \
                 VALUES (?, ?, 'channel', ?, ?, ?, 0, ?)",
                rusqlite::params![ok, cid, sender, body, ts, mg],
            );
        }
        Ok(())
    });

    // Build Message structs for the frontend
    decrypted
        .into_iter()
        .map(|(sender, body, ts, _mg)| {
            let is_own = sender == mpk;
            Message {
                id: 0, // temporary — will get real IDs on next query from SQLite
                sender_id: sender,
                body,
                timestamp: ts,
                is_own,
            }
        })
        .collect()
}

/// Remove a member from a community.
///
/// The caller must be the community owner or an admin to kick members.
/// Admins cannot kick other admins or the owner.
/// Sends `CommunityRequest::Kick` to the server, which removes the member
/// and rotates the MEK.
#[tauri::command]
pub async fn remove_community_member(
    community_id: String,
    pseudonym_key: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;

    // Check caller's role — use display role for backward-compat permission check
    let my_role = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .and_then(|c| c.my_role.clone())
            .unwrap_or_default()
    };

    if my_role != "owner" && my_role != "admin" {
        return Err(
            "insufficient permissions: must be owner or admin to remove members".to_string(),
        );
    }

    // Send Kick RPC to the community server (local validation passed)
    let response = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::Kick {
            target_pseudonym: pseudonym_key.clone(),
        },
    )
    .await?;

    // Check if server rejected the kick
    if let rekindle_protocol::messaging::CommunityResponse::Error { message, .. } = response {
        return Err(format!("server rejected kick: {message}"));
    }

    // Remove from local DB
    let community_id_clone = community_id.clone();
    let pseudonym_key_clone = pseudonym_key.clone();
    db_call(pool.inner(), move |conn| {
        conn.execute(
            "DELETE FROM community_members WHERE owner_key = ? AND community_id = ? AND pseudonym_key = ?",
            rusqlite::params![owner_key, community_id_clone, pseudonym_key_clone],
        )?;
        Ok(())
    })
    .await?;

    tracing::info!(
        community = %community_id,
        member = %pseudonym_key,
        "removed community member"
    );
    Ok(())
}

/// Get all role definitions for a community from the server.
#[tauri::command]
pub async fn get_roles(
    community_id: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<Vec<CommunityRoleDto>, String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    let response = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::GetRoles,
    )
    .await;

    match response {
        Ok(rekindle_protocol::messaging::CommunityResponse::RolesList { roles }) => {
            // Cache the roles locally in memory
            let role_defs: Vec<crate::state::RoleDefinition> = roles
                .iter()
                .map(crate::state::RoleDefinition::from_dto)
                .collect();
            {
                let mut communities = state.communities.write();
                if let Some(c) = communities.get_mut(&community_id) {
                    c.roles.clone_from(&role_defs);
                    c.my_role = Some(crate::state::display_role_name(&c.my_role_ids, &c.roles));
                }
            }
            // Persist to SQLite (DELETE + INSERT)
            let cid = community_id.clone();
            let defs = role_defs;
            db_call(pool.inner(), move |conn| {
                conn.execute(
                    "DELETE FROM community_roles WHERE owner_key = ? AND community_id = ?",
                    rusqlite::params![owner_key, cid],
                )?;
                for r in &defs {
                    conn.execute(
                        "INSERT INTO community_roles (owner_key, community_id, role_id, name, color, permissions, position, hoist, mentionable) \
                         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                        rusqlite::params![
                            owner_key, cid, r.id, r.name, r.color,
                            r.permissions.cast_signed(), r.position, i32::from(r.hoist), i32::from(r.mentionable),
                        ],
                    )?;
                }
                Ok(())
            }).await?;
            Ok(roles.iter().map(CommunityRoleDto::from).collect())
        }
        Ok(rekindle_protocol::messaging::CommunityResponse::Error { message, .. }) => {
            Err(format!("server rejected get_roles: {message}"))
        }
        Err(_) | Ok(_) => {
            // Return cached roles if server is unreachable
            let communities = state.communities.read();
            Ok(communities
                .get(&community_id)
                .map(|c| c.roles.iter().map(CommunityRoleDto::from).collect())
                .unwrap_or_default())
        }
    }
}

/// Create a new role in a community.
#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri IPC: each param is a distinct frontend field
pub async fn create_role(
    community_id: String,
    name: String,
    color: u32,
    permissions: u64,
    hoist: bool,
    mentionable: bool,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<u32, String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    let response = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::CreateRole {
            name: name.clone(),
            color,
            permissions,
            hoist,
            mentionable,
        },
    )
    .await;

    match response {
        Ok(rekindle_protocol::messaging::CommunityResponse::RoleCreated { role_id }) => {
            // Optimistic local state update
            let role_def = crate::state::RoleDefinition {
                id: role_id,
                name: name.clone(),
                color,
                permissions,
                position: 0, // server will assign real position via broadcast
                hoist,
                mentionable,
            };
            {
                let mut communities = state.communities.write();
                if let Some(c) = communities.get_mut(&community_id) {
                    c.roles.push(role_def);
                }
            }
            // Persist to SQLite
            let cid = community_id.clone();
            db_call(pool.inner(), move |conn| {
                conn.execute(
                    "INSERT OR REPLACE INTO community_roles (owner_key, community_id, role_id, name, color, permissions, position, hoist, mentionable) VALUES (?, ?, ?, ?, ?, ?, 0, ?, ?)",
                    rusqlite::params![owner_key, cid, role_id, name, color, permissions.cast_signed(), hoist, mentionable],
                )?;
                Ok(())
            }).await?;
            Ok(role_id)
        }
        Ok(rekindle_protocol::messaging::CommunityResponse::Error { message, .. }) => {
            Err(format!("server rejected create_role: {message}"))
        }
        Ok(_) => Err("unexpected response from server".into()),
        Err(e) => Err(e),
    }
}

/// Edit an existing role in a community.
#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri IPC: each param is a distinct frontend field
pub async fn edit_role(
    community_id: String,
    role_id: u32,
    name: Option<String>,
    color: Option<u32>,
    permissions: Option<u64>,
    position: Option<i32>,
    hoist: Option<bool>,
    mentionable: Option<bool>,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    let response = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::EditRole {
            role_id,
            name: name.clone(),
            color,
            permissions,
            position,
            hoist,
            mentionable,
        },
    )
    .await;

    match response {
        Ok(rekindle_protocol::messaging::CommunityResponse::Ok) => {
            // Optimistic local state update
            {
                let mut communities = state.communities.write();
                if let Some(c) = communities.get_mut(&community_id) {
                    if let Some(r) = c.roles.iter_mut().find(|r| r.id == role_id) {
                        if let Some(ref n) = name {
                            r.name.clone_from(n);
                        }
                        if let Some(col) = color {
                            r.color = col;
                        }
                        if let Some(p) = permissions {
                            r.permissions = p;
                        }
                        if let Some(pos) = position {
                            r.position = pos;
                        }
                        if let Some(h) = hoist {
                            r.hoist = h;
                        }
                        if let Some(m) = mentionable {
                            r.mentionable = m;
                        }
                    }
                    // Recompute display role in case permissions/name changed
                    c.my_role = Some(crate::state::display_role_name(&c.my_role_ids, &c.roles));
                }
            }
            // Persist to SQLite
            let cid = community_id.clone();
            db_call(pool.inner(), move |conn| {
                // Build dynamic UPDATE — only set fields that were provided
                let mut sets = Vec::new();
                let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
                if let Some(n) = name { sets.push("name = ?"); params.push(Box::new(n)); }
                if let Some(col) = color { sets.push("color = ?"); params.push(Box::new(col)); }
                if let Some(p) = permissions { sets.push("permissions = ?"); params.push(Box::new(p.cast_signed())); }
                if let Some(pos) = position { sets.push("position = ?"); params.push(Box::new(pos)); }
                if let Some(h) = hoist { sets.push("hoist = ?"); params.push(Box::new(h)); }
                if let Some(m) = mentionable { sets.push("mentionable = ?"); params.push(Box::new(m)); }
                if !sets.is_empty() {
                    let sql = format!(
                        "UPDATE community_roles SET {} WHERE owner_key = ? AND community_id = ? AND role_id = ?",
                        sets.join(", ")
                    );
                    params.push(Box::new(owner_key));
                    params.push(Box::new(cid));
                    params.push(Box::new(role_id));
                    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(std::convert::AsRef::as_ref).collect();
                    conn.execute(&sql, param_refs.as_slice())?;
                }
                Ok(())
            }).await?;
            Ok(())
        }
        Ok(rekindle_protocol::messaging::CommunityResponse::Error { message, .. }) => {
            Err(format!("server rejected edit_role: {message}"))
        }
        Ok(_) => Err("unexpected response from server".into()),
        Err(e) => Err(e),
    }
}

/// Delete a role from a community.
#[tauri::command]
pub async fn delete_role(
    community_id: String,
    role_id: u32,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    let response = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::DeleteRole { role_id },
    )
    .await;

    match response {
        Ok(rekindle_protocol::messaging::CommunityResponse::Ok) => {
            // Remove from in-memory state
            {
                let mut communities = state.communities.write();
                if let Some(c) = communities.get_mut(&community_id) {
                    c.roles.retain(|r| r.id != role_id);
                    c.my_role_ids.retain(|&id| id != role_id);
                    c.my_role = Some(crate::state::display_role_name(&c.my_role_ids, &c.roles));
                }
            }
            // Remove from SQLite + scrub from all members' role_ids
            let cid = community_id.clone();
            db_call(pool.inner(), move |conn| {
                conn.execute(
                    "DELETE FROM community_roles WHERE owner_key = ? AND community_id = ? AND role_id = ?",
                    rusqlite::params![owner_key, cid, role_id],
                )?;
                // Scrub the deleted role_id from all members' role_ids JSON
                let mut stmt = conn.prepare(
                    "SELECT pseudonym_key, role_ids FROM community_members WHERE owner_key = ? AND community_id = ?",
                )?;
                let members: Vec<(String, String)> = stmt.query_map(
                    rusqlite::params![owner_key, cid],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )?
                .filter_map(std::result::Result::ok)
                .collect();
                drop(stmt);
                let rid = role_id;
                for (pk, json) in &members {
                    let mut ids: Vec<u32> = serde_json::from_str(json).unwrap_or_default();
                    if ids.contains(&rid) {
                        ids.retain(|&id| id != rid);
                        let new_json = serde_json::to_string(&ids).unwrap_or_default();
                        conn.execute(
                            "UPDATE community_members SET role_ids = ? WHERE owner_key = ? AND community_id = ? AND pseudonym_key = ?",
                            rusqlite::params![new_json, owner_key, cid, pk],
                        )?;
                    }
                }
                // Also scrub from the communities.my_role_ids
                let my_ids_json: String = conn.query_row(
                    "SELECT my_role_ids FROM communities WHERE owner_key = ? AND id = ?",
                    rusqlite::params![owner_key, cid],
                    |row| row.get(0),
                ).unwrap_or_else(|_| "[0,1]".to_string());
                let mut my_ids: Vec<u32> = serde_json::from_str(&my_ids_json).unwrap_or_default();
                if my_ids.contains(&rid) {
                    my_ids.retain(|&id| id != rid);
                    let new_json = serde_json::to_string(&my_ids).unwrap_or_default();
                    conn.execute(
                        "UPDATE communities SET my_role_ids = ? WHERE owner_key = ? AND id = ?",
                        rusqlite::params![new_json, owner_key, cid],
                    )?;
                }
                Ok(())
            }).await?;
            Ok(())
        }
        Ok(rekindle_protocol::messaging::CommunityResponse::Error { message, .. }) => {
            Err(format!("server rejected delete_role: {message}"))
        }
        Ok(_) => Err("unexpected response from server".into()),
        Err(e) => Err(e),
    }
}

/// Assign a role to a member (additive — does not remove other roles).
#[tauri::command]
pub async fn assign_role(
    community_id: String,
    pseudonym_key: String,
    role_id: u32,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    let response = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::AssignRole {
            target_pseudonym: pseudonym_key.clone(),
            role_id,
        },
    )
    .await;

    match response {
        Ok(rekindle_protocol::messaging::CommunityResponse::Ok) => {
            // Update in-memory state if target is self
            let is_self = {
                let communities = state.communities.read();
                communities
                    .get(&community_id)
                    .is_some_and(|c| c.my_pseudonym_key.as_deref() == Some(&pseudonym_key))
            };
            if is_self {
                let mut communities = state.communities.write();
                if let Some(c) = communities.get_mut(&community_id) {
                    if !c.my_role_ids.contains(&role_id) {
                        c.my_role_ids.push(role_id);
                    }
                    c.my_role = Some(crate::state::display_role_name(&c.my_role_ids, &c.roles));
                }
            }
            // Update SQLite member role_ids
            let cid = community_id.clone();
            let pk = pseudonym_key.clone();
            db_call(pool.inner(), move |conn| {
                let current: String = conn.query_row(
                    "SELECT role_ids FROM community_members WHERE owner_key = ? AND community_id = ? AND pseudonym_key = ?",
                    rusqlite::params![owner_key, cid, pk],
                    |row| row.get(0),
                ).unwrap_or_else(|_| "[0,1]".to_string());
                let mut ids: Vec<u32> = serde_json::from_str(&current).unwrap_or_default();
                if !ids.contains(&role_id) {
                    ids.push(role_id);
                }
                let new_json = serde_json::to_string(&ids).unwrap_or_default();
                conn.execute(
                    "UPDATE community_members SET role_ids = ? WHERE owner_key = ? AND community_id = ? AND pseudonym_key = ?",
                    rusqlite::params![new_json, owner_key, cid, pk],
                )?;
                Ok(())
            }).await?;
            Ok(())
        }
        Ok(rekindle_protocol::messaging::CommunityResponse::Error { message, .. }) => {
            Err(format!("server rejected assign_role: {message}"))
        }
        Ok(_) => Err("unexpected response from server".into()),
        Err(e) => Err(e),
    }
}

/// Remove a role from a member.
#[tauri::command]
pub async fn unassign_role(
    community_id: String,
    pseudonym_key: String,
    role_id: u32,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    let response = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::UnassignRole {
            target_pseudonym: pseudonym_key.clone(),
            role_id,
        },
    )
    .await;

    match response {
        Ok(rekindle_protocol::messaging::CommunityResponse::Ok) => {
            // Update in-memory state if target is self
            let is_self = {
                let communities = state.communities.read();
                communities
                    .get(&community_id)
                    .is_some_and(|c| c.my_pseudonym_key.as_deref() == Some(&pseudonym_key))
            };
            if is_self {
                let mut communities = state.communities.write();
                if let Some(c) = communities.get_mut(&community_id) {
                    c.my_role_ids.retain(|&id| id != role_id);
                    c.my_role = Some(crate::state::display_role_name(&c.my_role_ids, &c.roles));
                }
            }
            // Update SQLite member role_ids
            let cid = community_id.clone();
            let pk = pseudonym_key.clone();
            db_call(pool.inner(), move |conn| {
                let current: String = conn.query_row(
                    "SELECT role_ids FROM community_members WHERE owner_key = ? AND community_id = ? AND pseudonym_key = ?",
                    rusqlite::params![owner_key, cid, pk],
                    |row| row.get(0),
                ).unwrap_or_else(|_| "[0,1]".to_string());
                let mut ids: Vec<u32> = serde_json::from_str(&current).unwrap_or_default();
                ids.retain(|&id| id != role_id);
                let new_json = serde_json::to_string(&ids).unwrap_or_default();
                conn.execute(
                    "UPDATE community_members SET role_ids = ? WHERE owner_key = ? AND community_id = ? AND pseudonym_key = ?",
                    rusqlite::params![new_json, owner_key, cid, pk],
                )?;
                Ok(())
            }).await?;
            Ok(())
        }
        Ok(rekindle_protocol::messaging::CommunityResponse::Error { message, .. }) => {
            Err(format!("server rejected unassign_role: {message}"))
        }
        Ok(_) => Err("unexpected response from server".into()),
        Err(e) => Err(e),
    }
}

/// Timeout a member (prevent sending for a duration).
#[tauri::command]
pub async fn timeout_member(
    community_id: String,
    pseudonym_key: String,
    duration_seconds: u64,
    reason: Option<String>,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    let response = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::TimeoutMember {
            target_pseudonym: pseudonym_key.clone(),
            duration_seconds,
            reason,
        },
    )
    .await;

    match response {
        Ok(rekindle_protocol::messaging::CommunityResponse::Ok) => {
            // Compute timeout_until and persist to SQLite
            let timeout_until = db::timestamp_now() / 1000 + duration_seconds.cast_signed();
            let cid = community_id.clone();
            let pk = pseudonym_key.clone();
            db_call(pool.inner(), move |conn| {
                conn.execute(
                    "UPDATE community_members SET timeout_until = ? WHERE owner_key = ? AND community_id = ? AND pseudonym_key = ?",
                    rusqlite::params![timeout_until, owner_key, cid, pk],
                )?;
                Ok(())
            }).await?;
            Ok(())
        }
        Ok(rekindle_protocol::messaging::CommunityResponse::Error { message, .. }) => {
            Err(format!("server rejected timeout_member: {message}"))
        }
        Ok(_) => Err("unexpected response from server".into()),
        Err(e) => Err(e),
    }
}

/// Remove a member's timeout.
#[tauri::command]
pub async fn remove_timeout(
    community_id: String,
    pseudonym_key: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    let response = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::RemoveTimeout {
            target_pseudonym: pseudonym_key.clone(),
        },
    )
    .await;

    match response {
        Ok(rekindle_protocol::messaging::CommunityResponse::Ok) => {
            // Clear timeout in SQLite
            let cid = community_id.clone();
            let pk = pseudonym_key.clone();
            db_call(pool.inner(), move |conn| {
                conn.execute(
                    "UPDATE community_members SET timeout_until = NULL WHERE owner_key = ? AND community_id = ? AND pseudonym_key = ?",
                    rusqlite::params![owner_key, cid, pk],
                )?;
                Ok(())
            }).await?;
            Ok(())
        }
        Ok(rekindle_protocol::messaging::CommunityResponse::Error { message, .. }) => {
            Err(format!("server rejected remove_timeout: {message}"))
        }
        Ok(_) => Err("unexpected response from server".into()),
        Err(e) => Err(e),
    }
}

/// Set a channel permission overwrite.
#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri IPC: each param is a distinct frontend field
pub async fn set_channel_overwrite(
    community_id: String,
    channel_id: String,
    target_type: String,
    target_id: String,
    allow: u64,
    deny: u64,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    let response = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::SetChannelOverwrite {
            channel_id: channel_id.clone(),
            target_type: target_type.clone(),
            target_id: target_id.clone(),
            allow,
            deny,
        },
    )
    .await;

    match response {
        Ok(rekindle_protocol::messaging::CommunityResponse::Ok) => {
            // Persist overwrite to local SQLite
            let comm_id = community_id.clone();
            let chan_id = channel_id.clone();
            let tgt_type = target_type.clone();
            let tgt_id = target_id.clone();
            db_call(pool.inner(), move |conn| {
                conn.execute(
                    "INSERT OR REPLACE INTO channel_overwrites (owner_key, community_id, channel_id, target_type, target_id, allow, deny) VALUES (?, ?, ?, ?, ?, ?, ?)",
                    rusqlite::params![owner_key, comm_id, chan_id, tgt_type, tgt_id, allow.cast_signed(), deny.cast_signed()],
                )?;
                Ok(())
            }).await?;
            Ok(())
        }
        Ok(rekindle_protocol::messaging::CommunityResponse::Error { message, .. }) => {
            Err(format!("server rejected set_channel_overwrite: {message}"))
        }
        Ok(_) => Err("unexpected response from server".into()),
        Err(e) => Err(e),
    }
}

/// Delete a channel permission overwrite.
#[tauri::command]
pub async fn delete_channel_overwrite(
    community_id: String,
    channel_id: String,
    target_type: String,
    target_id: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    let response = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::DeleteChannelOverwrite {
            channel_id: channel_id.clone(),
            target_type: target_type.clone(),
            target_id: target_id.clone(),
        },
    )
    .await;

    match response {
        Ok(rekindle_protocol::messaging::CommunityResponse::Ok) => {
            // Remove overwrite from local SQLite
            let comm_id = community_id.clone();
            let chan_id = channel_id.clone();
            let tgt_type = target_type.clone();
            let tgt_id = target_id.clone();
            db_call(pool.inner(), move |conn| {
                conn.execute(
                    "DELETE FROM channel_overwrites WHERE owner_key = ? AND community_id = ? AND channel_id = ? AND target_type = ? AND target_id = ?",
                    rusqlite::params![owner_key, comm_id, chan_id, tgt_type, tgt_id],
                )?;
                Ok(())
            }).await?;
            Ok(())
        }
        Ok(rekindle_protocol::messaging::CommunityResponse::Error { message, .. }) => Err(format!(
            "server rejected delete_channel_overwrite: {message}"
        )),
        Ok(_) => Err("unexpected response from server".into()),
        Err(e) => Err(e),
    }
}

/// Delete a channel from a community.
///
/// Sends `CommunityRequest::DeleteChannel` to the server, then removes
/// the channel from local state and `SQLite`.
#[tauri::command]
pub async fn delete_channel(
    community_id: String,
    channel_id: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;

    let response = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::DeleteChannel {
            channel_id: channel_id.clone(),
        },
    )
    .await?;

    if let rekindle_protocol::messaging::CommunityResponse::Error { message, .. } = response {
        return Err(format!("server rejected channel deletion: {message}"));
    }

    // Remove from local state
    {
        let mut communities = state.communities.write();
        if let Some(community) = communities.get_mut(&community_id) {
            community.channels.retain(|ch| ch.id != channel_id);
        }
    }

    // Remove from local SQLite
    let community_id_clone = community_id.clone();
    let channel_id_clone = channel_id.clone();
    db_call(pool.inner(), move |conn| {
        conn.execute(
            "DELETE FROM channels WHERE owner_key = ? AND id = ? AND community_id = ?",
            rusqlite::params![owner_key, channel_id_clone, community_id_clone],
        )?;
        Ok(())
    })
    .await?;

    tracing::info!(community = %community_id, channel = %channel_id, "channel deleted");
    Ok(())
}

/// Rename a channel in a community.
#[tauri::command]
pub async fn rename_channel(
    community_id: String,
    channel_id: String,
    new_name: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;

    let response = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::RenameChannel {
            channel_id: channel_id.clone(),
            new_name: new_name.clone(),
        },
    )
    .await?;

    if let rekindle_protocol::messaging::CommunityResponse::Error { message, .. } = response {
        return Err(format!("server rejected channel rename: {message}"));
    }

    // Update local state
    {
        let mut communities = state.communities.write();
        if let Some(community) = communities.get_mut(&community_id) {
            if let Some(ch) = community.channels.iter_mut().find(|ch| ch.id == channel_id) {
                ch.name.clone_from(&new_name);
            }
        }
    }

    // Update local SQLite
    let community_id_clone = community_id.clone();
    let channel_id_clone = channel_id.clone();
    let name_clone = new_name.clone();
    db_call(pool.inner(), move |conn| {
        conn.execute(
            "UPDATE channels SET name = ? WHERE owner_key = ? AND id = ? AND community_id = ?",
            rusqlite::params![name_clone, owner_key, channel_id_clone, community_id_clone],
        )?;
        Ok(())
    })
    .await?;

    tracing::info!(community = %community_id, channel = %channel_id, "channel renamed");
    Ok(())
}

/// Update community metadata (name, description).
#[tauri::command]
pub async fn update_community_info(
    community_id: String,
    name: Option<String>,
    description: Option<String>,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;

    let response = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::UpdateCommunity {
            name: name.clone(),
            description: description.clone(),
        },
    )
    .await;

    match response {
        Ok(rekindle_protocol::messaging::CommunityResponse::CommunityUpdated) => {}
        Ok(rekindle_protocol::messaging::CommunityResponse::Error { message, .. }) => {
            return Err(format!("server rejected community update: {message}"));
        }
        Ok(_) => {
            return Err("unexpected response from server".into());
        }
        Err(e) => {
            return Err(e);
        }
    }

    // Update local state
    {
        let mut communities = state.communities.write();
        if let Some(community) = communities.get_mut(&community_id) {
            if let Some(ref n) = name {
                community.name.clone_from(n);
            }
            if let Some(ref d) = description {
                community.description = Some(d.clone());
            }
        }
    }

    // Update local SQLite
    let cid = community_id.clone();
    db_call(pool.inner(), move |conn| {
        if let Some(ref n) = name {
            conn.execute(
                "UPDATE communities SET name = ? WHERE owner_key = ? AND id = ?",
                rusqlite::params![n, owner_key, cid],
            )?;
        }
        if let Some(ref d) = description {
            conn.execute(
                "UPDATE communities SET description = ? WHERE owner_key = ? AND id = ?",
                rusqlite::params![d, owner_key, cid],
            )?;
        }
        Ok(())
    })
    .await?;

    tracing::info!(community = %community_id, "community info updated");
    Ok(())
}

/// Ban a member from a community.
#[tauri::command]
pub async fn ban_member(
    community_id: String,
    pseudonym_key: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let response = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::Ban {
            target_pseudonym: pseudonym_key.clone(),
        },
    )
    .await?;

    if let rekindle_protocol::messaging::CommunityResponse::Error { message, .. } = response {
        return Err(format!("server rejected ban: {message}"));
    }

    // Remove from local member list (server already kicked them)
    {
        let mut communities = state.communities.write();
        if let Some(community) = communities.get_mut(&community_id) {
            // Members are stored in community.members in the SolidJS store,
            // but on the Rust side this is in the DB — the frontend will
            // update its store via the handler.
            let _ = community;
        }
    }

    tracing::info!(community = %community_id, member = %pseudonym_key, "member banned");
    Ok(())
}

/// Unban a member from a community.
#[tauri::command]
pub async fn unban_member(
    community_id: String,
    pseudonym_key: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let response = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::Unban {
            target_pseudonym: pseudonym_key.clone(),
        },
    )
    .await?;

    if let rekindle_protocol::messaging::CommunityResponse::Error { message, .. } = response {
        return Err(format!("server rejected unban: {message}"));
    }

    tracing::info!(community = %community_id, member = %pseudonym_key, "member unbanned");
    Ok(())
}

/// Banned member info for the frontend.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BannedMemberInfo {
    pub pseudonym_key: String,
    pub display_name: String,
    pub banned_at: u64,
}

/// Get the list of banned members for a community.
#[tauri::command]
pub async fn get_ban_list(
    community_id: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<Vec<BannedMemberInfo>, String> {
    let response = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::GetBanList,
    )
    .await;

    match response {
        Ok(rekindle_protocol::messaging::CommunityResponse::BanList { banned }) => Ok(banned
            .into_iter()
            .map(|b| BannedMemberInfo {
                pseudonym_key: b.pseudonym_key,
                display_name: b.display_name,
                banned_at: b.banned_at,
            })
            .collect()),
        Ok(rekindle_protocol::messaging::CommunityResponse::Error { message, .. }) => {
            Err(format!("server rejected ban list request: {message}"))
        }
        Ok(_) => Err("unexpected response from server".into()),
        Err(e) => Err(e),
    }
}

/// Force MEK rotation for a community.
#[tauri::command]
pub async fn rotate_mek(
    community_id: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let response = send_community_rpc(
        state.inner(),
        pool.inner(),
        &community_id,
        rekindle_protocol::messaging::CommunityRequest::RotateMEK,
    )
    .await?;

    if let rekindle_protocol::messaging::CommunityResponse::Error { message, .. } = response {
        return Err(format!("server rejected MEK rotation: {message}"));
    }

    tracing::info!(community = %community_id, "MEK rotation requested");
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
    // Get our own pseudonym key to identify ourselves in the member list
    let my_pseudonym = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .and_then(|c| c.my_pseudonym_key.clone())
    };
    // Get our own status to show for ourselves
    let my_status =
        state_helpers::identity_status(state.inner()).unwrap_or(crate::state::UserStatus::Online);

    // Get cached role definitions for display name computation
    let role_defs = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .map(|c| c.roles.clone())
            .unwrap_or_default()
    };

    let owner_key = state_helpers::current_owner_key(state.inner())?;
    let community_id_clone = community_id.clone();
    let members = db_call(pool.inner(), move |conn| {
        let mut stmt = conn.prepare(
            "SELECT pseudonym_key, display_name, role_ids, timeout_until FROM community_members \
                 WHERE owner_key = ? AND community_id = ? ORDER BY display_name",
        )?;

        let rows = stmt.query_map(rusqlite::params![owner_key, community_id_clone], |row| {
            let pseudonym_key = db::get_str(row, "pseudonym_key");

            // Pseudonym keys are per-community and unlinkable to real identity,
            // so we can't cross-reference with the friends list for presence.
            // Show our own real status; other members default to online
            // (presence tracking via server is a future enhancement).
            let status_str = if my_pseudonym.as_deref() == Some(&pseudonym_key) {
                match my_status {
                    crate::state::UserStatus::Online => "online",
                    crate::state::UserStatus::Away => "away",
                    crate::state::UserStatus::Busy => "busy",
                    crate::state::UserStatus::Offline => "offline",
                }
            } else {
                "online" // default for other members — server presence tracking TODO
            };

            let role_ids_json = db::get_str(row, "role_ids");
            let role_ids: Vec<u32> =
                serde_json::from_str(&role_ids_json).unwrap_or_else(|_| vec![0, 1]);
            let display_role = crate::state::display_role_name(&role_ids, &role_defs);
            let timeout_until: Option<u64> = row
                .get::<_, Option<i64>>("timeout_until")
                .ok()
                .flatten()
                .map(i64::cast_unsigned);

            Ok(MemberDto {
                pseudonym_key,
                display_name: db::get_str(row, "display_name"),
                role_ids,
                display_role,
                status: status_str.to_string(),
                timeout_until,
            })
        })?;

        let mut members = Vec::new();
        for row in rows {
            members.push(row?);
        }
        Ok(members)
    })
    .await?;

    Ok(members)
}

/// Ensure the community server is running and knows about this community.
///
/// If the server is not running, spawns it. If it is already running, sends an
/// IPC `HostCommunity` command for the new community. Then registers the creator
/// as the first member via IPC Join, and fetches the server's route blob from DHT
/// in the background for remote clients.
async fn ensure_community_hosted(app: &tauri::AppHandle, state: &SharedState, community_id: &str) {
    // Gather community data + creator pseudonym for the HostCommunity IPC.
    // The creator is registered atomically during host_community on the server,
    // eliminating the race condition where a separate Join RPC would need to
    // arrive after the (slow) hosting process completes.
    let community_data = {
        let communities = state.communities.read();
        communities.get(community_id).and_then(|c| {
            let dht_key = c.dht_record_key.as_ref()?;
            let keypair = c.dht_owner_keypair.as_ref()?;
            let pseudonym = c.my_pseudonym_key.clone().unwrap_or_default();
            Some((
                c.id.clone(),
                dht_key.clone(),
                keypair.clone(),
                c.name.clone(),
                pseudonym,
            ))
        })
    };

    let creator_display_name = state
        .identity
        .read()
        .as_ref()
        .map(|id| id.display_name.clone())
        .unwrap_or_default();

    if state.server_process.lock().is_some() {
        // Server already running — send IPC to host this new community
        if let Some((cid, dht_key, keypair, nm, pseudonym)) = community_data.clone() {
            let cdn = creator_display_name.clone();
            let socket_path = crate::ipc_client::default_socket_path();
            let result = tokio::task::spawn_blocking(move || {
                crate::ipc_client::host_community_blocking(
                    &socket_path,
                    &cid,
                    &dht_key,
                    &keypair,
                    &nm,
                    &pseudonym,
                    &cdn,
                    5,
                )
            })
            .await;
            match result {
                Ok(Ok(())) => {
                    tracing::info!(community = %community_id, "community hosted via IPC (creator registered)");
                }
                Ok(Err(e)) => {
                    tracing::error!(community = %community_id, error = %e, "HostCommunity IPC failed");
                }
                Err(e) => {
                    tracing::error!(community = %community_id, error = %e, "HostCommunity IPC task panicked");
                }
            }
        }
    } else {
        // First community — spawn the server (it will host all owned communities
        // from DB on startup, but the new community needs an explicit HostCommunity).
        super::auth::maybe_spawn_server(app, state);

        // The server needs a moment to start up. Send HostCommunity with retries.
        if let Some((cid, dht_key, keypair, nm, pseudonym)) = community_data.clone() {
            let cdn = creator_display_name.clone();
            let socket_path = crate::ipc_client::default_socket_path();
            let result = tokio::task::spawn_blocking(move || {
                crate::ipc_client::host_community_blocking(
                    &socket_path,
                    &cid,
                    &dht_key,
                    &keypair,
                    &nm,
                    &pseudonym,
                    &cdn,
                    10,
                )
            })
            .await;
            match result {
                Ok(Ok(())) => {
                    tracing::info!(community = %community_id, "community hosted via IPC after server spawn");
                }
                Ok(Err(e)) => {
                    tracing::error!(community = %community_id, error = %e, "HostCommunity IPC failed after spawn");
                }
                Err(e) => {
                    tracing::error!(community = %community_id, error = %e, "HostCommunity task panicked");
                }
            }
        }
    }

    // Fetch the server's route blob from DHT in the background.
    // Remote clients need this to send Veilid app_call to the server.
    // This is NOT needed for the creator's IPC-based communication.
    let bg_state = state.clone();
    let bg_app = app.clone();
    let bg_community_id = community_id.to_string();
    tauri::async_runtime::spawn(async move {
        fetch_and_persist_route_blob(&bg_app, &bg_state, &bg_community_id).await;
    });
}

/// Fetch the server's route blob from DHT and persist it.
///
/// This is run in the background after community creation. Remote clients
/// need the route blob to send Veilid `app_call` to the server, but the
/// creator's IPC-based communication doesn't need it.
async fn fetch_and_persist_route_blob(
    app: &tauri::AppHandle,
    state: &SharedState,
    community_id: &str,
) {
    // Generous retry: server needs to start, attach to Veilid, open DHT, publish route
    for attempt in 0..20u32 {
        if attempt > 0 {
            let delay = std::time::Duration::from_secs((3 * u64::from(attempt)).min(30));
            tokio::time::sleep(delay).await;
        }
        if let Some(blob) = fetch_server_route_blob(state, community_id).await {
            {
                let mut communities = state.communities.write();
                if let Some(c) = communities.get_mut(community_id) {
                    c.server_route_blob = Some(blob.clone());
                }
            }
            persist_server_route_blob(app, community_id, &blob);
            tracing::info!(
                community = %community_id, attempt,
                "server route blob fetched and persisted"
            );
            return;
        }
    }
    tracing::error!(
        community = %community_id,
        "failed to fetch server route blob after 20 attempts"
    );
}

/// Fetch the community server's route blob from DHT subkey 6.
///
/// Retries a few times with delay since the server may still be publishing
/// after startup. Returns `None` if the route blob is not yet available.
async fn fetch_server_route_blob(state: &SharedState, community_id: &str) -> Option<Vec<u8>> {
    let dht_record_key = {
        let communities = state.communities.read();
        communities.get(community_id)?.dht_record_key.clone()?
    };

    let routing_context = {
        let node = state.node.read();
        let nh = node.as_ref()?;
        nh.routing_context.clone()
    };

    // The server needs time to start: up to 30s for Veilid attach + 5 DHT
    // retries with exponential backoff. Use generous retry timing here.
    for attempt in 0..10u32 {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        }

        let mgr = rekindle_protocol::dht::DHTManager::new(routing_context.clone());
        if mgr.open_record(&dht_record_key).await.is_err() {
            continue;
        }

        let blob = mgr
            .get_value(
                &dht_record_key,
                rekindle_protocol::dht::community::SUBKEY_SERVER_ROUTE,
            )
            .await
            .ok()
            .flatten();

        let _ = mgr.close_record(&dht_record_key).await;

        if blob.is_some() {
            tracing::debug!(
                community = %community_id,
                attempt,
                "fetched server route blob from DHT"
            );
            return blob;
        }
    }

    tracing::warn!(
        community = %community_id,
        "server route blob not available after retries"
    );
    None
}

/// Persist a community's `server_route_blob` to `SQLite`.
///
/// Called when the route blob is first fetched from DHT (after creation) or
/// when a DHT watch notifies us of a route change. Ensures the route survives
/// logout/restart without needing to re-fetch from DHT.
pub(crate) fn persist_server_route_blob(
    app: &tauri::AppHandle,
    community_id: &str,
    route_blob: &[u8],
) {
    use tauri::Manager as _;
    let pool: tauri::State<'_, db::DbPool> = app.state();
    let state: tauri::State<'_, SharedState> = app.state();
    let owner_key = state
        .identity
        .read()
        .as_ref()
        .map(|id| id.public_key.clone())
        .unwrap_or_default();
    let cid = community_id.to_string();
    let blob = route_blob.to_vec();
    db_fire(pool.inner(), "persist server_route_blob", move |conn| {
        conn.execute(
            "UPDATE communities SET server_route_blob = ?1 WHERE owner_key = ?2 AND id = ?3",
            rusqlite::params![blob, owner_key, cid],
        )?;
        Ok(())
    });
}
