use std::sync::Arc;
use std::time::Duration;

use rekindle_protocol::dht::community::{
    OverwriteType, PermissionOverwrite, RoleDefinition,
    SUBKEY_CHANNELS, SUBKEY_MEK, SUBKEY_MEMBERS, SUBKEY_METADATA, SUBKEY_SERVER_ROUTE,
    ROLE_EVERYONE_ID, permissions,
};
use rekindle_protocol::dht::DHTManager;
use rusqlite::params;
use tokio::sync::mpsc;

use crate::mek;
use crate::server_state::{HostedCommunity, ServerChannel, ServerMember, ServerState};

/// Load members for a community from the server database.
fn load_members_from_db(
    state: &Arc<ServerState>,
    community_id: &str,
) -> Result<Vec<ServerMember>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let mut stmt = db
        .prepare(
            "SELECT pseudonym_key_hex, display_name, joined_at, route_blob FROM server_members WHERE community_id = ?",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![community_id], |row| {
            Ok(ServerMember {
                pseudonym_key_hex: row.get(0)?,
                display_name: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                role_ids: Vec::new(), // filled below
                joined_at: row.get(2)?,
                route_blob: row.get(3)?,
                timeout_until: None,  // filled below
            })
        })
        .map_err(|e| e.to_string())?;
    let mut members: Vec<ServerMember> = rows.filter_map(Result::ok).collect();

    // Load role_ids from junction table
    {
        let mut role_stmt = db
            .prepare("SELECT pseudonym_key_hex, role_id FROM server_member_roles WHERE community_id = ?")
            .map_err(|e| e.to_string())?;
        let role_rows = role_stmt
            .query_map(params![community_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?))
            })
            .map_err(|e| e.to_string())?;
        for row in role_rows.flatten() {
            if let Some(member) = members.iter_mut().find(|m| m.pseudonym_key_hex == row.0) {
                member.role_ids.push(row.1);
            }
        }
    }

    // Load timeouts
    {
        let mut to_stmt = db
            .prepare("SELECT pseudonym_key_hex, timeout_until FROM server_member_timeouts WHERE community_id = ?")
            .map_err(|e| e.to_string())?;
        let to_rows = to_stmt
            .query_map(params![community_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
            })
            .map_err(|e| e.to_string())?;
        let now = timestamp_now_secs();
        for row in to_rows.flatten() {
            if row.1 > now {
                if let Some(member) = members.iter_mut().find(|m| m.pseudonym_key_hex == row.0) {
                    member.timeout_until = Some(row.1);
                }
            }
        }
    }

    Ok(members)
}

/// Load channels for a community from the server database.
fn load_channels_from_db(
    state: &Arc<ServerState>,
    community_id: &str,
) -> Result<Vec<ServerChannel>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let mut stmt = db
        .prepare(
            "SELECT id, name, channel_type, sort_order FROM server_channels WHERE community_id = ?",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![community_id], |row| {
            Ok(ServerChannel {
                id: row.get(0)?,
                name: row.get(1)?,
                channel_type: row.get(2)?,
                sort_order: row.get(3)?,
                permission_overwrites: Vec::new(), // filled below
            })
        })
        .map_err(|e| e.to_string())?;
    let mut channels: Vec<ServerChannel> = rows.filter_map(Result::ok).collect();

    // Load permission overwrites
    {
        let mut ow_stmt = db
            .prepare(
                "SELECT channel_id, target_type, target_id, allow_bits, deny_bits \
                 FROM server_channel_overwrites WHERE community_id = ?",
            )
            .map_err(|e| e.to_string())?;
        let ow_rows = ow_stmt
            .query_map(params![community_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, u64>(3)?,
                    row.get::<_, u64>(4)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        for row in ow_rows.flatten() {
            if let Some(ch) = channels.iter_mut().find(|c| c.id == row.0) {
                let target_type = match row.1.as_str() {
                    "member" => OverwriteType::Member,
                    _ => OverwriteType::Role,
                };
                ch.permission_overwrites.push(PermissionOverwrite {
                    target_type,
                    target_id: row.2,
                    allow: row.3,
                    deny: row.4,
                });
            }
        }
    }

    Ok(channels)
}

/// Load role definitions for a community from the server database.
fn load_roles_from_db(
    state: &Arc<ServerState>,
    community_id: &str,
) -> Result<Vec<RoleDefinition>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let mut stmt = db
        .prepare(
            "SELECT id, name, color, permissions, position, hoist, mentionable \
             FROM server_roles WHERE community_id = ? ORDER BY position DESC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![community_id], |row| {
            Ok(RoleDefinition {
                id: row.get(0)?,
                name: row.get(1)?,
                color: row.get(2)?,
                permissions: row.get(3)?,
                position: row.get(4)?,
                hoist: row.get::<_, i32>(5)? != 0,
                mentionable: row.get::<_, i32>(6)? != 0,
            })
        })
        .map_err(|e| e.to_string())?;
    Ok(rows.filter_map(Result::ok).collect())
}

/// Create the 5 default roles for a new community and persist to DB.
pub fn create_default_roles(
    state: &Arc<ServerState>,
    community_id: &str,
) -> Result<Vec<RoleDefinition>, String> {
    let default_roles = vec![
        RoleDefinition {
            id: ROLE_EVERYONE_ID,
            name: "@everyone".to_string(),
            color: 0,
            permissions: permissions::everyone_permissions(),
            position: 0,
            hoist: false,
            mentionable: false,
        },
        RoleDefinition {
            id: 1,
            name: "Member".to_string(),
            color: 0,
            permissions: permissions::member_permissions(),
            position: 1,
            hoist: false,
            mentionable: false,
        },
        RoleDefinition {
            id: 2,
            name: "Moderator".to_string(),
            color: 0x0034_98DB, // blue
            permissions: permissions::moderator_permissions(),
            position: 2,
            hoist: true,
            mentionable: true,
        },
        RoleDefinition {
            id: 3,
            name: "Admin".to_string(),
            color: 0x00E7_4C3C, // red
            permissions: permissions::admin_permissions(),
            position: 3,
            hoist: true,
            mentionable: true,
        },
        RoleDefinition {
            id: 4,
            name: "Owner".to_string(),
            color: 0x00F1_C40F, // gold
            permissions: permissions::owner_permissions(),
            position: 4,
            hoist: true,
            mentionable: false,
        },
    ];

    let db = state.db.lock().map_err(|e| e.to_string())?;
    for role in &default_roles {
        db.execute(
            "INSERT OR IGNORE INTO server_roles (community_id, id, name, color, permissions, position, hoist, mentionable) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                community_id,
                role.id,
                role.name,
                role.color,
                role.permissions.cast_signed(),
                role.position,
                i32::from(role.hoist),
                i32::from(role.mentionable),
            ],
        )
        .map_err(|e| format!("failed to insert default role: {e}"))?;
    }

    Ok(default_roles)
}

/// Open a DHT record and allocate a private route for the community.
///
/// Returns `(route_id, route_blob)` — both `None` if allocation fails.
///
/// The DHT open is retried with backoff because the server's Veilid node may
/// still be attaching to the network when this is called. The record was created
/// by the client's Veilid node (a separate process), so this node needs network
/// access to discover and open it.
async fn setup_dht_and_route(
    state: &Arc<ServerState>,
    community_id: &str,
    dht_record_key: &str,
    owner_keypair_hex: &str,
) -> Result<(Option<veilid_core::RouteId>, Option<Vec<u8>>, bool), String> {
    let keypair = parse_owner_keypair(owner_keypair_hex)?;

    // Wait for Veilid to be attached before trying DHT operations.
    // The server may have just started and the node may still be connecting.
    let max_wait = 30;
    for attempt in 0..max_wait {
        match state.api.get_state().await {
            Ok(vs) if vs.attachment.state.is_attached() => break,
            _ => {
                if attempt == max_wait - 1 {
                    tracing::warn!(
                        community = %community_id,
                        "Veilid not attached after {max_wait}s — proceeding anyway"
                    );
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }

    // Open DHT record with write access, retrying with backoff.
    // The record was created by the client node, so our server node may need
    // a moment to discover it on the network.
    let mgr = DHTManager::new(state.routing_context.clone());
    let mut dht_opened = false;
    for attempt in 0..5 {
        match mgr.open_record_writable(dht_record_key, keypair.clone()).await {
            Ok(()) => {
                tracing::debug!(community = %community_id, "opened DHT record with write access");
                dht_opened = true;
                break;
            }
            Err(e) => {
                if attempt < 4 {
                    let delay = Duration::from_secs(2u64.pow(attempt));
                    tracing::debug!(
                        error = %e,
                        community = %community_id,
                        attempt = attempt + 1,
                        "retrying DHT record open in {:?}",
                        delay,
                    );
                    tokio::time::sleep(delay).await;
                } else {
                    tracing::warn!(
                        error = %e,
                        community = %community_id,
                        "failed to open DHT record after 5 attempts — DHT writes will fail"
                    );
                }
            }
        }
    }

    // Allocate a private route for this community
    let (route_id, route_blob) = match state.api.new_private_route().await {
        Ok(rb) => {
            tracing::info!(community = %community_id, "allocated private route for community");
            (Some(rb.route_id), Some(rb.blob))
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                community = %community_id,
                "failed to allocate private route — community will use polling fallback"
            );
            (None, None)
        }
    };

    // Publish route blob to DHT subkey 6 (only if record is open)
    if dht_opened {
        if let Some(ref blob) = route_blob {
            let mgr2 = DHTManager::new(state.routing_context.clone());
            if let Err(e) = mgr2
                .set_value(dht_record_key, SUBKEY_SERVER_ROUTE, blob.clone())
                .await
            {
                tracing::warn!(error = %e, "failed to publish server route to DHT");
            }
        }
    }

    Ok((route_id, route_blob, dht_opened))
}

/// Start hosting a community: load/create state, allocate route, publish to DHT.
///
/// The `creator_pseudonym_key` is registered as the first member with owner
/// permissions. This avoids a race condition where a separate Join RPC might
/// arrive before the community is fully hosted.
#[allow(clippy::too_many_lines)]
pub async fn host_community(
    state: &Arc<ServerState>,
    community_id: &str,
    dht_record_key: &str,
    owner_keypair_hex: &str,
    name: &str,
    creator_pseudonym_key: &str,
    creator_display_name: &str,
) -> Result<(), String> {
    // Check if already hosted
    {
        let hosted = state.hosted.read();
        if hosted.contains_key(community_id) {
            return Ok(());
        }
    }

    // Persist community to DB FIRST — child tables (server_mek, server_members,
    // server_channels) have FK constraints referencing hosted_communities(id),
    // so the parent row must exist before any child inserts.
    {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        if let Err(e) = db.execute(
            "INSERT OR IGNORE INTO hosted_communities (id, dht_record_key, owner_keypair_hex, name, created_at) VALUES (?,?,?,?,?)",
            params![community_id, dht_record_key, owner_keypair_hex, name, timestamp_now_secs()],
        ) {
            tracing::error!(error = %e, community = %community_id, "failed to persist hosted community to DB");
        }
    }

    let mek_val = mek::load_latest_mek(state, community_id)
        .unwrap_or_else(|| mek::create_initial_mek(state, community_id));

    let channels = load_channels_from_db(state, community_id)?;

    // Load or create default roles
    let mut roles = load_roles_from_db(state, community_id)?;
    if roles.is_empty() {
        roles = create_default_roles(state, community_id)?;
    }

    let mut members = load_members_from_db(state, community_id)?;

    let (route_id, route_blob, dht_opened) =
        setup_dht_and_route(state, community_id, dht_record_key, owner_keypair_hex).await?;

    // Load description and creator_pseudonym from DB
    let (description, mut creator_pseudonym_hex) = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let desc = db
            .query_row(
                "SELECT description FROM hosted_communities WHERE id = ?",
                params![community_id],
                |row| row.get::<_, String>(0),
            )
            .unwrap_or_default();
        let creator = db
            .query_row(
                "SELECT creator_pseudonym FROM hosted_communities WHERE id = ?",
                params![community_id],
                |row| row.get::<_, String>(0),
            )
            .unwrap_or_default();
        (desc, creator)
    };

    // If a creator pseudonym key was provided and the creator isn't already
    // registered (i.e. this is a brand-new community), register them atomically
    // as the first member with owner permissions. This prevents the race
    // condition where a separate Join RPC would need to arrive after hosting
    // completes.
    if !creator_pseudonym_key.is_empty()
        && creator_pseudonym_hex.is_empty()
        && !members.iter().any(|m| m.pseudonym_key_hex == creator_pseudonym_key)
    {
        #[allow(clippy::cast_possible_wrap)]
        let now = timestamp_now_secs() as i64;
        let owner_role_ids = vec![ROLE_EVERYONE_ID, 1, 2, 3, 4];

        // Persist creator to DB
        {
            let db = state.db.lock().map_err(|e| e.to_string())?;
            let _ = db.execute(
                "INSERT OR IGNORE INTO server_members (community_id, pseudonym_key_hex, display_name, joined_at) VALUES (?,?,?,?)",
                params![community_id, creator_pseudonym_key, creator_display_name, now],
            );
            for role_id in &owner_role_ids {
                let _ = db.execute(
                    "INSERT OR IGNORE INTO server_member_roles (community_id, pseudonym_key_hex, role_id) VALUES (?,?,?)",
                    params![community_id, creator_pseudonym_key, role_id],
                );
            }
            let _ = db.execute(
                "UPDATE hosted_communities SET creator_pseudonym = ? WHERE id = ?",
                params![creator_pseudonym_key, community_id],
            );
        }

        creator_pseudonym_hex = creator_pseudonym_key.to_string();
        members.push(ServerMember {
            pseudonym_key_hex: creator_pseudonym_key.to_string(),
            display_name: creator_display_name.to_string(),
            role_ids: owner_role_ids,
            joined_at: now,
            route_blob: None,
            timeout_until: None,
        });

        tracing::info!(
            community = %community_id,
            creator = %creator_pseudonym_key,
            "registered creator as first member during host_community"
        );
    }

    let community = HostedCommunity {
        community_id: community_id.to_string(),
        dht_record_key: dht_record_key.to_string(),
        owner_keypair_hex: owner_keypair_hex.to_string(),
        name: name.to_string(),
        description,
        route_id,
        route_blob,
        mek: mek_val,
        members,
        channels,
        roles,
        creator_pseudonym_hex,
    };

    state
        .hosted
        .write()
        .insert(community_id.to_string(), community);

    // Publish all subkeys so clients can discover this community.
    // Only attempt DHT writes if the record was successfully opened.
    if dht_opened {
        publish_metadata(state, community_id, name).await;
        publish_channels(state, community_id).await;
        publish_member_roster(state, community_id).await;
        publish_mek_bundle(state, community_id).await;
    } else {
        tracing::warn!(
            community = %community_id,
            "skipping initial DHT publication — record not open (keepalive will retry)"
        );
    }

    tracing::info!(
        community = %community_id,
        dht_key = %dht_record_key,
        owner = %&owner_keypair_hex[..16.min(owner_keypair_hex.len())],
        "now hosting community"
    );
    Ok(())
}

/// Stop hosting a community: remove from state, release route, delete from DB.
pub fn unhost_community(state: &Arc<ServerState>, community_id: &str) {
    let removed = state.hosted.write().remove(community_id);
    if let Some(community) = removed {
        if let Some(route_id) = community.route_id {
            let _ = state.api.release_private_route(route_id);
        }
        // Remove from DB so it's not re-loaded on restart
        // CASCADE FKs clean up server_members, server_channels, server_mek
        if let Ok(db) = state.db.lock() {
            if let Err(e) = db.execute(
                "DELETE FROM hosted_communities WHERE id = ?",
                rusqlite::params![community_id],
            ) {
                tracing::warn!(error = %e, community = %community_id, "failed to delete hosted community from DB");
            }
        }
        tracing::info!(community = %community_id, "stopped hosting community");
    }
}

/// DHT keep-alive loop: re-writes all subkeys every 2 minutes to prevent expiration.
///
/// Veilid private routes have a TTL of ~5 minutes. By re-allocating every 2 minutes
/// we ensure routes are always fresh before they expire.
pub async fn dht_keepalive_loop(
    state: Arc<ServerState>,
    mut shutdown_rx: mpsc::Receiver<()>,
) {
    let mut interval = tokio::time::interval(Duration::from_secs(120));

    loop {
        tokio::select! {
            _ = interval.tick() => {
                // Check Veilid attachment before attempting DHT writes
                match state.api.get_state().await {
                    Ok(veilid_state) if veilid_state.attachment.state.is_attached() => {
                        rewrite_all_communities(&state).await;
                    }
                    Ok(_) => {
                        tracing::warn!("skipping DHT keepalive: Veilid not attached");
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "skipping DHT keepalive: failed to query Veilid state");
                    }
                }
            }
            _ = shutdown_rx.recv() => {
                tracing::info!("DHT keep-alive loop shutting down");
                break;
            }
        }
    }
}

/// Data needed per-community during a keepalive cycle.
struct KeepaliveData {
    community_id: String,
    dht_key: String,
    owner_keypair: String,
    route_blob: Option<Vec<u8>>,
    name: String,
}

/// Re-allocate a fresh private route for a community during keepalive.
///
/// Releases the old route, allocates a new one, and updates in-memory state.
/// Returns the blob to publish (or the existing blob as fallback).
async fn keepalive_refresh_route(
    state: &Arc<ServerState>,
    entry: &KeepaliveData,
) -> Option<Vec<u8>> {
    // Atomically take the old route_id under a write lock so that
    // `handle_server_route_change` can't race and double-release.
    let old_route_id = {
        let mut hosted = state.hosted.write();
        hosted
            .get_mut(&entry.community_id)
            .and_then(|c| c.route_id.take())
    };
    // Release outside the lock (best-effort — may already be expired)
    if let Some(old_id) = old_route_id {
        let _ = state.api.release_private_route(old_id);
    }

    match state.api.new_private_route().await {
        Ok(rb) => {
            tracing::info!(
                community = %entry.community_id,
                "keepalive: re-allocated private route"
            );
            let mut hosted = state.hosted.write();
            if let Some(c) = hosted.get_mut(&entry.community_id) {
                c.route_id = Some(rb.route_id);
                c.route_blob = Some(rb.blob.clone());
            }
            Some(rb.blob)
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                community = %entry.community_id,
                "keepalive: route re-allocation failed — keeping existing blob"
            );
            entry.route_blob.clone()
        }
    }
}

/// Re-write DHT records for all hosted communities to prevent expiration.
async fn rewrite_all_communities(state: &Arc<ServerState>) {
    // Collect data outside the lock (parking_lot guards are !Send)
    let community_data: Vec<KeepaliveData> = {
        let hosted = state.hosted.read();
        hosted
            .values()
            .map(|c| {
                let name = state.db.lock().ok()
                    .and_then(|db| {
                        db.query_row(
                            "SELECT name FROM hosted_communities WHERE id = ?",
                            params![c.community_id],
                            |row| row.get::<_, String>(0),
                        ).ok()
                    })
                    .unwrap_or_default();
                KeepaliveData {
                    community_id: c.community_id.clone(),
                    dht_key: c.dht_record_key.clone(),
                    owner_keypair: c.owner_keypair_hex.clone(),
                    route_blob: c.route_blob.clone(),
                    name,
                }
            })
            .collect()
    };

    let mgr = DHTManager::new(state.routing_context.clone());

    for entry in &community_data {
        // Re-open the DHT record with write access before writing.
        match parse_owner_keypair(&entry.owner_keypair) {
            Ok(keypair) => {
                if let Err(e) = mgr.open_record_writable(&entry.dht_key, keypair).await {
                    tracing::warn!(
                        error = %e,
                        community = %entry.community_id,
                        "failed to re-open DHT record with write access during keepalive"
                    );
                    continue;
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, community = %entry.community_id, "invalid owner keypair");
                continue;
            }
        }

        // Always re-allocate a fresh route (Veilid route TTL ~5 min, RouteChange can be missed)
        let route_blob_to_publish = keepalive_refresh_route(state, entry).await;
        if let Some(ref blob) = route_blob_to_publish {
            if let Err(e) = mgr
                .set_value(&entry.dht_key, SUBKEY_SERVER_ROUTE, blob.clone())
                .await
            {
                tracing::warn!(
                    error = %e,
                    community = %entry.community_id,
                    "failed to refresh server route in DHT"
                );
            }
        }

        publish_metadata(state, &entry.community_id, &entry.name).await;
        publish_channels(state, &entry.community_id).await;
        publish_member_roster(state, &entry.community_id).await;

        tracing::debug!(community = %entry.community_id, "DHT keep-alive refresh done");
    }
}

/// Handle a route change event: re-allocate routes for affected communities.
pub async fn handle_server_route_change(state: &Arc<ServerState>, dead_routes: &[veilid_core::RouteId]) {
    // Collect affected community IDs outside the lock (parking_lot guards are !Send)
    let affected: Vec<(String, String)> = {
        let hosted = state.hosted.read();
        hosted
            .values()
            .filter(|c| {
                c.route_id
                    .as_ref()
                    .is_some_and(|rid| dead_routes.contains(rid))
            })
            .map(|c| (c.community_id.clone(), c.dht_record_key.clone()))
            .collect()
    };

    for (community_id, dht_key) in affected {
        tracing::info!(community = %community_id, "re-allocating dead route");

        // Atomically take the dead route_id under a write lock to prevent
        // double-release races with keepalive_refresh_route.
        // Don't call release_private_route — the route is already dead
        // (reported via RouteChange). Releasing a dead route produces
        // an "Invalid argument" error from the Veilid API.
        {
            let mut hosted = state.hosted.write();
            if let Some(c) = hosted.get_mut(&community_id) {
                c.route_id = None;
            }
        }

        match state.api.new_private_route().await {
            Ok(rb) => {
                let new_blob = rb.blob;

                // Update in-memory state
                {
                    let mut hosted = state.hosted.write();
                    if let Some(community) = hosted.get_mut(&community_id) {
                        community.route_id = Some(rb.route_id);
                        community.route_blob = Some(new_blob.clone());
                    }
                }

                // Publish new route to DHT
                let mgr = DHTManager::new(state.routing_context.clone());
                if let Err(e) = mgr
                    .set_value(&dht_key, SUBKEY_SERVER_ROUTE, new_blob)
                    .await
                {
                    tracing::warn!(error = %e, community = %community_id, "failed to publish new route to DHT");
                }
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    community = %community_id,
                    "failed to re-allocate route"
                );
                let mut hosted = state.hosted.write();
                if let Some(community) = hosted.get_mut(&community_id) {
                    community.route_id = None;
                    community.route_blob = None;
                }
            }
        }
    }
}

/// Clear member `route_blob` fields for routes that Veilid reports as dead.
///
/// When imported remote routes die, broadcast attempts to those members will fail.
/// By clearing their route blobs, we avoid futile send attempts until the member
/// re-joins or sends a new route update.
pub fn clear_dead_member_routes(
    _state: &Arc<ServerState>,
    dead_remote_routes: &[veilid_core::RouteId],
) {
    // We can't directly compare RouteId to route_blob bytes — route blobs are
    // the serialized form that gets imported. Instead, we track which route_ids
    // we've imported for members. For now, log the event; a full implementation
    // would maintain a RouteId→(community_id, pseudonym) cache.
    //
    // As a practical measure, we clear route_blobs for members whose routes
    // fail during broadcast (handled in broadcast_to_members error path).
    if !dead_remote_routes.is_empty() {
        tracing::info!(
            count = dead_remote_routes.len(),
            "remote routes died — affected member routes will fail on next broadcast"
        );
    }
}

/// Publish community metadata (name, description) to DHT subkey 0.
pub async fn publish_metadata(state: &Arc<ServerState>, community_id: &str, name: &str) {
    let (dht_key, owner_public_key) = {
        let hosted = state.hosted.read();
        let Some(community) = hosted.get(community_id) else {
            return;
        };
        // Extract the owner's public key from the stored keypair string
        let owner_key = parse_owner_keypair(&community.owner_keypair_hex)
            .map(|kp| kp.key().to_string())
            .unwrap_or_default();
        (community.dht_record_key.clone(), owner_key)
    };

    let now = timestamp_now_secs();
    let metadata = rekindle_protocol::dht::community::CommunityMetadata {
        name: name.to_string(),
        description: None,
        icon_hash: None,
        created_at: now,
        owner_key: owner_public_key,
        last_refreshed: now,
    };
    let data = serde_json::to_vec(&metadata).unwrap_or_default();

    let mgr = DHTManager::new(state.routing_context.clone());
    if let Err(e) = mgr.set_value(&dht_key, SUBKEY_METADATA, data).await {
        tracing::warn!(error = %e, community = %community_id, "failed to publish metadata to DHT");
    }
}

/// Publish the channel list to DHT subkey 1.
pub async fn publish_channels(state: &Arc<ServerState>, community_id: &str) {
    let (dht_key, channels_json) = {
        let hosted = state.hosted.read();
        let Some(community) = hosted.get(community_id) else {
            return;
        };
        let wrapper = serde_json::json!({
            "channels": community.channels.iter().map(|ch| {
                serde_json::json!({
                    "id": ch.id,
                    "name": ch.name,
                    "channelType": ch.channel_type,
                    "sortOrder": ch.sort_order,
                })
            }).collect::<Vec<_>>(),
            "lastRefreshed": timestamp_now_secs(),
        });
        (
            community.dht_record_key.clone(),
            serde_json::to_vec(&wrapper).unwrap_or_default(),
        )
    };

    let mgr = DHTManager::new(state.routing_context.clone());
    if let Err(e) = mgr
        .set_value(&dht_key, SUBKEY_CHANNELS, channels_json)
        .await
    {
        tracing::warn!(error = %e, community = %community_id, "failed to publish channels to DHT");
    }
}

/// Publish the current member roster to DHT subkey 2.
///
/// Serializes all members with their pseudonym key, role, display name, and join time.
pub async fn publish_member_roster(state: &Arc<ServerState>, community_id: &str) {
    let (dht_key, roster_json) = {
        let hosted = state.hosted.read();
        let Some(community) = hosted.get(community_id) else {
            return;
        };

        let wrapper = serde_json::json!({
            "members": community.members.iter().map(|m| {
                serde_json::json!({
                    "pseudonymKey": m.pseudonym_key_hex,
                    "roleIds": m.role_ids,
                    "displayName": m.display_name,
                    "joinedAt": m.joined_at,
                })
            }).collect::<Vec<_>>(),
            "lastRefreshed": timestamp_now_secs(),
        });

        (
            community.dht_record_key.clone(),
            serde_json::to_vec(&wrapper).unwrap_or_default(),
        )
    };

    let mgr = DHTManager::new(state.routing_context.clone());
    if let Err(e) = mgr
        .set_value(&dht_key, SUBKEY_MEMBERS, roster_json)
        .await
    {
        tracing::warn!(
            error = %e,
            community = %community_id,
            "failed to publish member roster to DHT"
        );
    }
}

/// Publish MEK generation metadata to DHT subkey 5.
///
/// Only the generation number and a refresh timestamp are written (no key material).
/// Clients detect generation changes via DHT watch and then request the actual
/// key bytes from the server via `CommunityRequest::RequestMEK`.
pub async fn publish_mek_bundle(state: &Arc<ServerState>, community_id: &str) {
    let (dht_key, mek_data) = {
        let hosted = state.hosted.read();
        let Some(community) = hosted.get(community_id) else {
            return;
        };
        let bundle = serde_json::json!({
            "generation": community.mek.generation(),
            "lastRefreshed": timestamp_now_secs(),
        });
        (
            community.dht_record_key.clone(),
            serde_json::to_vec(&bundle).unwrap_or_default(),
        )
    };

    let mgr = DHTManager::new(state.routing_context.clone());
    if let Err(e) = mgr.set_value(&dht_key, SUBKEY_MEK, mek_data).await {
        tracing::warn!(error = %e, community = %community_id, "failed to publish MEK bundle to DHT");
    }
}

/// Parse an owner keypair from its serialized string representation.
///
/// Veilid's `KeyPair` implements `FromStr` with the format produced by `Display`.
fn parse_owner_keypair(hex_str: &str) -> Result<veilid_core::KeyPair, String> {
    hex_str
        .parse::<veilid_core::KeyPair>()
        .map_err(|e| format!("invalid owner keypair: {e}"))
}

fn timestamp_now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
