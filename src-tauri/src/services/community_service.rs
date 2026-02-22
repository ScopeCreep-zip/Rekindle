use std::sync::Arc;

use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_protocol::dht::community::{SUBKEY_CHANNELS, SUBKEY_METADATA, SUBKEY_SERVER_ROUTE};
use rekindle_protocol::dht::DHTManager;

use crate::state::{AppState, ChannelInfo, ChannelType, CommunityState, RoleDefinition};
use crate::state_helpers;

/// Create a new community and publish it to DHT.
pub async fn create_community(state: &Arc<AppState>, name: &str) -> Result<String, String> {
    // Clone routing context out of the parking_lot lock before any .await
    let routing_context = state_helpers::routing_context(state);

    // Try DHT-backed creation first
    if let Some(rc) = routing_context {
        if let Some(id) = create_community_with_dht(state, &rc, name).await? {
            return Ok(id);
        }
    } else {
        tracing::debug!("node not attached, creating community locally only");
    }

    // Fallback: create community without DHT record (e.g. node not connected yet)
    let community_id = format!("community_{}", hex::encode(rand_bytes(16)));
    create_community_local(state, &community_id, name);
    Ok(community_id)
}

/// Attempt to create a community with a DHT record. Returns `Some(id)` on success.
async fn create_community_with_dht(
    state: &Arc<AppState>,
    routing_context: &veilid_core::RoutingContext,
    name: &str,
) -> Result<Option<String>, String> {
    let mgr = DHTManager::new(routing_context.clone());
    let (key, owner_keypair) = match mgr.create_record(7).await {
        Ok(result) => result,
        Err(e) => {
            tracing::warn!(error = %e, "failed to create DHT record for community, proceeding locally");
            return Ok(None);
        }
    };

    // Publish community metadata to subkey 0
    let metadata = serde_json::json!({ "name": name, "description": null });
    let metadata_bytes =
        serde_json::to_vec(&metadata).map_err(|e| format!("failed to serialize metadata: {e}"))?;
    if let Err(e) = mgr.set_value(&key, SUBKEY_METADATA, metadata_bytes).await {
        tracing::warn!(error = %e, "failed to publish community metadata to DHT");
    }

    // Publish initial channel list to subkey 1
    let default_channel = ChannelInfo {
        id: format!("channel_{}", hex::encode(rand_bytes(8))),
        name: "general".to_string(),
        channel_type: ChannelType::Text,
        unread_count: 0,
    };
    let channels_json =
        crate::state::serialize_channel_list_for_dht(std::slice::from_ref(&default_channel), 0);
    let channels_bytes = serde_json::to_vec(&channels_json)
        .map_err(|e| format!("failed to serialize channels: {e}"))?;
    if let Err(e) = mgr.set_value(&key, SUBKEY_CHANNELS, channels_bytes).await {
        tracing::warn!(error = %e, "failed to publish channel list to DHT");
    }

    tracing::info!(dht_key = %key, "community DHT record created");

    let mek = MediaEncryptionKey::generate(1);
    let mek_generation = mek.generation();
    tracing::debug!(community = %key, mek_generation, "generated initial MEK for community");

    let my_pseudonym_key = derive_pseudonym_key(state, &key);
    state.mek_cache.lock().insert(key.clone(), mek);
    let dht_owner_keypair = owner_keypair.map(|kp| kp.to_string());

    let community = CommunityState {
        id: key.clone(),
        name: name.to_string(),
        description: None,
        channels: vec![default_channel],
        my_role_ids: vec![0, 1, 2, 3, 4], // @everyone, member, moderator, admin, owner
        roles: default_roles(),
        my_role: Some("owner".to_string()),
        dht_record_key: Some(key.clone()),
        dht_owner_keypair,
        my_pseudonym_key,
        mek_generation,
        server_route_blob: None,
        is_hosted: true,
    };

    state.communities.write().insert(key.clone(), community);
    tracing::info!(name = %name, dht_key = %key, "community created with DHT record");
    Ok(Some(key))
}

/// Create a community in local state only (no DHT).
fn create_community_local(state: &Arc<AppState>, community_id: &str, name: &str) {
    let default_channel = ChannelInfo {
        id: format!("channel_{}", hex::encode(rand_bytes(8))),
        name: "general".to_string(),
        channel_type: ChannelType::Text,
        unread_count: 0,
    };

    let mek = MediaEncryptionKey::generate(1);
    let mek_generation = mek.generation();
    tracing::debug!(community = %community_id, mek_generation, "generated initial MEK for community (local only)");

    let my_pseudonym_key = derive_pseudonym_key(state, community_id);
    state.mek_cache.lock().insert(community_id.to_string(), mek);

    let community = CommunityState {
        id: community_id.to_string(),
        name: name.to_string(),
        description: None,
        channels: vec![default_channel],
        my_role_ids: vec![0, 1, 2, 3, 4], // @everyone, member, moderator, admin, owner
        roles: default_roles(),
        my_role: Some("owner".to_string()),
        dht_record_key: None,
        dht_owner_keypair: None,
        my_pseudonym_key,
        mek_generation,
        server_route_blob: None,
        is_hosted: true,
    };

    state
        .communities
        .write()
        .insert(community_id.to_string(), community);
    tracing::info!(community = %community_id, name = %name, "community created (local only)");
}

/// Derive the pseudonym public key hex for a community from the identity secret.
fn derive_pseudonym_key(state: &Arc<AppState>, community_id: &str) -> Option<String> {
    let secret = state.identity_secret.lock();
    secret.as_ref().map(|s| {
        let signing_key =
            rekindle_crypto::group::pseudonym::derive_community_pseudonym(s, community_id);
        hex::encode(signing_key.verifying_key().to_bytes())
    })
}

/// Join an existing community by ID or invite code.
///
/// Reads community metadata from DHT, then sends a `CommunityRequest::Join`
/// RPC to the community server via `app_call`. On success, the server returns
/// the MEK, channel list, and assigned role.
pub async fn join_community(state: &Arc<AppState>, community_id: &str) -> Result<(), String> {
    let routing_context = state_helpers::routing_context(state);

    let (name, description, mut channels, dht_record_key, server_route_blob) =
        read_community_from_dht(routing_context.as_ref(), community_id).await;

    let my_pseudonym_key = derive_pseudonym_key(state, community_id);
    let our_display_name = state_helpers::identity_display_name(state);

    // Get our route blob so the server can broadcast to us
    let our_route_blob = state_helpers::our_route_blob(state);

    // --- Send CommunityRequest::Join RPC to server ---
    let mut mek_generation = 0u64;
    let mut role = "member".to_string();
    let mut role_ids = vec![0u32, 1]; // default: @everyone + members
    let mut roles = default_roles();

    let identity_secret = { *state.identity_secret.lock() };
    if let (Some(ref route_blob), Some(ref rc), Some(secret)) =
        (&server_route_blob, &routing_context, identity_secret)
    {
        let join_params = JoinRpcParams {
            identity_secret: secret,
            community_id: community_id.to_string(),
            my_pseudonym_key: my_pseudonym_key.clone(),
            display_name: our_display_name,
            our_route_blob: &our_route_blob,
        };
        match send_join_rpc(state, rc, route_blob, &join_params).await {
            Ok(Some(result)) => {
                mek_generation = result.mek_generation;
                role = result.role;
                role_ids = result.role_ids;
                if !result.roles.is_empty() {
                    roles = result.roles;
                }
                if !result.channels.is_empty() {
                    channels = result.channels;
                }
            }
            Ok(None) => {}           // RPC failed gracefully, join locally
            Err(e) => return Err(e), // Server explicitly rejected
        }
    } else if server_route_blob.is_none() {
        tracing::debug!(community = %community_id, "no server route blob — local join only");
    }

    let community = CommunityState {
        id: community_id.to_string(),
        name,
        description,
        channels,
        my_role_ids: role_ids,
        roles,
        my_role: Some(role),
        dht_record_key,
        dht_owner_keypair: None,
        my_pseudonym_key,
        mek_generation,
        server_route_blob,
        is_hosted: false,
    };

    state
        .communities
        .write()
        .insert(community_id.to_string(), community);

    tracing::info!(community = %community_id, "joined community");
    Ok(())
}

/// Read community metadata, channels, and server route from DHT.
///
/// Returns `(name, description, channels, dht_record_key, server_route_blob)`.
async fn read_community_from_dht(
    routing_context: Option<&veilid_core::RoutingContext>,
    community_id: &str,
) -> (
    String,
    Option<String>,
    Vec<ChannelInfo>,
    Option<String>,
    Option<Vec<u8>>,
) {
    let Some(rc) = routing_context else {
        return (
            default_community_name(community_id),
            None,
            vec![],
            None,
            None,
        );
    };

    let mgr = DHTManager::new(rc.clone());

    if let Err(e) = mgr.open_record(community_id).await {
        tracing::warn!(error = %e, "failed to open community DHT record, joining locally");
        return (
            default_community_name(community_id),
            None,
            vec![],
            None,
            None,
        );
    }

    let (name, description) = match mgr.get_value(community_id, SUBKEY_METADATA).await {
        Ok(Some(data)) => parse_community_metadata(&data, community_id),
        Ok(None) => (default_community_name(community_id), None),
        Err(e) => {
            tracing::warn!(error = %e, "failed to read community metadata from DHT");
            (default_community_name(community_id), None)
        }
    };

    let channels = match mgr.get_value(community_id, SUBKEY_CHANNELS).await {
        Ok(Some(data)) => crate::state::parse_dht_channel_list(&data),
        Ok(None) | Err(_) => vec![],
    };

    // Watch metadata(0), channels(1), roster(2), roles(3), MEK bundles(5), server route(6)
    if let Err(e) = mgr.watch_record(community_id, &[0, 1, 2, 3, 5, 6]).await {
        tracing::warn!(error = %e, "failed to watch community DHT record");
    }

    let dht_key = community_id.to_string();
    let server_route_blob = mgr
        .get_value(&dht_key, SUBKEY_SERVER_ROUTE)
        .await
        .ok()
        .flatten();

    (
        name,
        description,
        channels,
        Some(dht_key),
        server_route_blob,
    )
}

/// Result of a successful join RPC to the community server.
struct JoinRpcResult {
    mek_generation: u64,
    role: String,
    role_ids: Vec<u32>,
    roles: Vec<RoleDefinition>,
    channels: Vec<ChannelInfo>,
}

/// Parameters for sending a join RPC to the community server.
struct JoinRpcParams<'a> {
    identity_secret: [u8; 32],
    community_id: String,
    my_pseudonym_key: Option<String>,
    display_name: String,
    our_route_blob: &'a Option<Vec<u8>>,
}

/// Send a `CommunityRequest::Join` RPC to the server.
///
/// Returns `Ok(Some(result))` on success, `Ok(None)` on graceful failure,
/// or `Err` if the server explicitly rejected the join.
async fn send_join_rpc(
    state: &Arc<AppState>,
    routing_context: &veilid_core::RoutingContext,
    server_route_blob: &[u8],
    params: &JoinRpcParams<'_>,
) -> Result<Option<JoinRpcResult>, String> {
    let signing_key = rekindle_crypto::group::pseudonym::derive_community_pseudonym(
        &params.identity_secret,
        &params.community_id,
    );

    let request = rekindle_protocol::messaging::CommunityRequest::Join {
        pseudonym_pubkey: params.my_pseudonym_key.clone().unwrap_or_default(),
        invite_code: None,
        display_name: params.display_name.clone(),
        prekey_bundle: Vec::new(),
        route_blob: params.our_route_blob.clone(),
    };
    let request_bytes = serde_json::to_vec(&request)
        .map_err(|e| format!("failed to serialize join request: {e}"))?;

    let timestamp = crate::db::timestamp_now().cast_unsigned();
    let envelope = rekindle_protocol::messaging::sender::build_envelope(
        &signing_key,
        timestamp,
        rand_nonce(),
        request_bytes,
    );

    let route_id = match state_helpers::import_route_blob(state, server_route_blob) {
        Ok(rid) => rid,
        Err(e) => {
            tracing::warn!(error = %e, "failed to import server route — joining locally");
            return Ok(None);
        }
    };

    let call_result =
        rekindle_protocol::messaging::sender::send_call(routing_context, route_id, &envelope).await;

    let response_bytes = match call_result {
        Ok(bytes) => bytes,
        Err(e) => {
            tracing::warn!(error = %e, "failed to send join RPC to server — joining locally");
            return Ok(None);
        }
    };

    parse_join_response(state, params, &response_bytes)
}

fn parse_join_response(
    state: &Arc<AppState>,
    params: &JoinRpcParams<'_>,
    response_bytes: &[u8],
) -> Result<Option<JoinRpcResult>, String> {
    match serde_json::from_slice::<rekindle_protocol::messaging::CommunityResponse>(response_bytes)
    {
        Ok(rekindle_protocol::messaging::CommunityResponse::Joined {
            mek_encrypted,
            mek_generation,
            channels: server_channels,
            role_ids,
            roles: server_roles,
        }) => {
            let role = crate::state::display_role_name(
                &role_ids,
                &server_roles
                    .iter()
                    .map(RoleDefinition::from_dto)
                    .collect::<Vec<_>>(),
            );
            tracing::info!(
                community = %params.community_id, role = %role,
                mek_generation, channels = server_channels.len(),
                "joined community via server RPC"
            );

            let channels = server_channels
                .into_iter()
                .map(|ch| ChannelInfo {
                    id: ch.id,
                    name: ch.name,
                    channel_type: ch.channel_type.parse().unwrap_or(ChannelType::Text),
                    unread_count: 0,
                })
                .collect();

            let roles = server_roles.iter().map(RoleDefinition::from_dto).collect();

            if let Some(mek) = MediaEncryptionKey::from_wire_bytes(&mek_encrypted) {
                tracing::debug!(community = %params.community_id, generation = mek.generation(), "MEK received and cached");
                state
                    .mek_cache
                    .lock()
                    .insert(params.community_id.clone(), mek);
            }

            Ok(Some(JoinRpcResult {
                mek_generation,
                role,
                role_ids,
                roles,
                channels,
            }))
        }
        Ok(rekindle_protocol::messaging::CommunityResponse::Error { message, .. }) => {
            tracing::warn!(error = %message, "server rejected join request");
            Err(format!("server rejected join: {message}"))
        }
        Ok(other) => {
            tracing::warn!(?other, "unexpected response from server");
            Ok(None)
        }
        Err(e) => {
            tracing::warn!(error = %e, "failed to parse server join response");
            Ok(None)
        }
    }
}

fn rand_nonce() -> Vec<u8> {
    use rand::RngCore;
    let mut nonce = vec![0u8; 24];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    nonce
}

/// Create a new channel within a community.
///
/// Only users with "owner" or "admin" role can create channels.
pub async fn create_channel(
    state: &Arc<AppState>,
    community_id: &str,
    channel_name: &str,
    channel_type: &str,
) -> Result<String, String> {
    // Permission-based access check and collect current channels + DHT key
    let (existing_channels, dht_record_key, is_hosted) = {
        use rekindle_protocol::dht::community::permissions;

        let communities = state.communities.read();
        let community = communities
            .get(community_id)
            .ok_or_else(|| format!("community {community_id} not found"))?;

        // Compute base permissions by OR'ing all role permissions for our role IDs
        let my_perms = community.my_role_ids.iter().fold(0u64, |acc, role_id| {
            community
                .roles
                .iter()
                .find(|r| r.id == *role_id)
                .map_or(acc, |r| acc | r.permissions)
        });
        if !permissions::has_permission(my_perms, permissions::MANAGE_CHANNELS) {
            return Err(
                "insufficient permissions: you do not have MANAGE_CHANNELS permission".to_string(),
            );
        }

        (
            community.channels.clone(),
            community.dht_record_key.clone(),
            community.is_hosted,
        )
    };

    let channel_id = format!("channel_{}", hex::encode(rand_bytes(8)));

    let channel = ChannelInfo {
        id: channel_id.clone(),
        name: channel_name.to_string(),
        channel_type: channel_type.parse().unwrap_or(ChannelType::Text),
        unread_count: 0,
    };

    // Add to community state
    {
        let communities = state.communities.read();
        if !communities.contains_key(community_id) {
            return Err(format!("community {community_id} not found"));
        }
    }
    state_helpers::push_community_channel(state, community_id, channel);

    // Update community DHT record subkey 1 (channel list).
    // Only write to DHT for non-hosted communities — hosted communities have
    // their DHT records managed exclusively by the rekindle-server process
    // (which holds the owner keypair). The client does NOT have write access.
    if !is_hosted {
        if let Some(dht_key) = &dht_record_key {
            let routing_context = state_helpers::routing_context(state);

            if let Some(rc) = routing_context {
                let mgr = DHTManager::new(rc);

                let mut all_channels = existing_channels;
                all_channels.push(ChannelInfo {
                    id: channel_id.clone(),
                    name: channel_name.to_string(),
                    channel_type: channel_type.parse().unwrap_or(ChannelType::Text),
                    unread_count: 0,
                });

                let channels_wrapper =
                    crate::state::serialize_channel_list_for_dht(&all_channels, 0);
                let channels_bytes = serde_json::to_vec(&channels_wrapper)
                    .map_err(|e| format!("failed to serialize channels: {e}"))?;

                if let Err(e) = mgr
                    .set_value(dht_key, SUBKEY_CHANNELS, channels_bytes)
                    .await
                {
                    tracing::warn!(error = %e, "failed to update channel list in DHT");
                }
            }
        }
    }

    tracing::info!(
        community = %community_id,
        channel = %channel_id,
        name = %channel_name,
        "channel created"
    );
    Ok(channel_id)
}

/// Parse community metadata JSON from DHT value bytes.
fn parse_community_metadata(data: &[u8], community_id: &str) -> (String, Option<String>) {
    if let Ok(metadata) = serde_json::from_slice::<serde_json::Value>(data) {
        let name = metadata
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or(&community_id[..8.min(community_id.len())])
            .to_string();
        let desc = metadata
            .get("description")
            .and_then(|v| v.as_str())
            .map(String::from);
        (name, desc)
    } else {
        (default_community_name(community_id), None)
    }
}

/// Parse channel list JSON from DHT value bytes.
/// Construct a default community display name from a (potentially long) ID.
fn default_community_name(community_id: &str) -> String {
    format!("Community {}", &community_id[..8.min(community_id.len())])
}

/// Default role definitions for a newly created community.
///
/// Mirrors the server's `DEFAULT_ROLES` so the creator sees roles immediately
/// without needing to round-trip to the server.
fn default_roles() -> Vec<RoleDefinition> {
    use rekindle_protocol::dht::community::permissions;
    vec![
        RoleDefinition {
            id: 0,
            name: "@everyone".to_string(),
            color: 0,
            permissions: permissions::everyone_permissions(),
            position: 0,
            hoist: false,
            mentionable: false,
        },
        RoleDefinition {
            id: 1,
            name: "Members".to_string(),
            color: 0,
            permissions: permissions::member_permissions(),
            position: 1,
            hoist: false,
            mentionable: false,
        },
        RoleDefinition {
            id: 2,
            name: "Moderator".to_string(),
            color: 0x0034_98DB, // blue — matches server
            permissions: permissions::moderator_permissions(),
            position: 2,
            hoist: true,
            mentionable: true,
        },
        RoleDefinition {
            id: 3,
            name: "Admin".to_string(),
            color: 0x00E7_4C3C, // red — matches server
            permissions: permissions::admin_permissions(),
            position: 3,
            hoist: true,
            mentionable: true,
        },
        RoleDefinition {
            id: 4,
            name: "Owner".to_string(),
            color: 0x00F1_C40F, // gold — matches server
            permissions: permissions::owner_permissions(),
            position: 4,
            hoist: true,
            mentionable: false,
        },
    ]
}

fn rand_bytes(len: usize) -> Vec<u8> {
    use rand::RngCore;
    let mut bytes = vec![0u8; len];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes
}
