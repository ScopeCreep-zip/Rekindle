use std::sync::Arc;

use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_protocol::dht::DHTManager;

use crate::state::{AppState, ChannelInfo, ChannelType, CommunityState};

/// Minimum role required to create a channel in a community.
const ROLE_CAN_CREATE_CHANNEL: &[&str] = &["owner", "admin"];

/// Community DHT subkey layout:
///   0 = metadata (name, description, icon)
///   1 = channel list
///   2 = member list
///   3 = roles
///   4 = invites
///   5 = MEK (encrypted)
const SUBKEY_METADATA: u32 = 0;
const SUBKEY_CHANNELS: u32 = 1;
const SUBKEY_MEMBERS: u32 = 2;

/// Create a new community and publish it to DHT.
pub async fn create_community(
    state: &Arc<AppState>,
    name: &str,
) -> Result<String, String> {
    // Generate a unique ID for the community
    let community_id = format!("community_{}", hex::encode(rand_bytes(16)));

    // Clone routing context out of the parking_lot lock before any .await
    let routing_context = {
        let node = state.node.read();
        node.as_ref()
            .filter(|nh| nh.is_attached)
            .map(|nh| nh.routing_context.clone())
    };

    // Create DHT record with 6 subkeys: metadata, channels, members, roles, invites, MEK
    let dht_record_key = if let Some(rc) = routing_context {
        let mgr = DHTManager::new(rc);
        match mgr.create_record(6).await {
            Ok((key, _owner_keypair)) => {
                // Publish community metadata to subkey 0
                let metadata = serde_json::json!({
                    "name": name,
                    "description": null,
                });
                let metadata_bytes = serde_json::to_vec(&metadata)
                    .map_err(|e| format!("failed to serialize metadata: {e}"))?;
                if let Err(e) = mgr.set_value(&key, SUBKEY_METADATA, metadata_bytes).await {
                    tracing::warn!(error = %e, "failed to publish community metadata to DHT");
                }

                // Publish initial channel list to subkey 1
                let default_channel_id = format!("channel_{}", hex::encode(rand_bytes(8)));
                let channels_json = serde_json::json!([{
                    "id": default_channel_id,
                    "name": "general",
                    "channelType": "text",
                }]);
                let channels_bytes = serde_json::to_vec(&channels_json)
                    .map_err(|e| format!("failed to serialize channels: {e}"))?;
                if let Err(e) = mgr.set_value(&key, SUBKEY_CHANNELS, channels_bytes).await {
                    tracing::warn!(error = %e, "failed to publish channel list to DHT");
                }

                tracing::info!(dht_key = %key, "community DHT record created");

                // Build channel from the ID we published
                let default_channel = ChannelInfo {
                    id: default_channel_id,
                    name: "general".to_string(),
                    channel_type: ChannelType::Text,
                    unread_count: 0,
                };

                let community = CommunityState {
                    id: community_id.clone(),
                    name: name.to_string(),
                    description: None,
                    channels: vec![default_channel],
                    my_role: Some("owner".to_string()),
                    dht_record_key: Some(key.clone()),
                };

                // Generate MEK for the community (generation 1 = initial key)
                let mek = MediaEncryptionKey::generate(1);
                tracing::debug!(
                    community = %community_id,
                    mek_generation = mek.generation(),
                    "generated initial MEK for community"
                );
                // TODO: Store MEK in Stronghold: VAULT_COMMUNITIES / mek_{community_id}
                // TODO: Distribute MEK to members via their individual Signal sessions
                drop(mek);

                state
                    .communities
                    .write()
                    .insert(community_id.clone(), community);

                tracing::info!(community = %community_id, name = %name, dht_key = %key, "community created with DHT record");
                return Ok(community_id);
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to create DHT record for community, proceeding locally");
                None
            }
        }
    } else {
        tracing::debug!("node not attached, creating community locally only");
        None
    };

    // Fallback: create community without DHT record (e.g. node not connected yet)
    let default_channel = ChannelInfo {
        id: format!("channel_{}", hex::encode(rand_bytes(8))),
        name: "general".to_string(),
        channel_type: ChannelType::Text,
        unread_count: 0,
    };

    let community = CommunityState {
        id: community_id.clone(),
        name: name.to_string(),
        description: None,
        channels: vec![default_channel],
        my_role: Some("owner".to_string()),
        dht_record_key,
    };

    let mek = MediaEncryptionKey::generate(1);
    tracing::debug!(
        community = %community_id,
        mek_generation = mek.generation(),
        "generated initial MEK for community"
    );
    // TODO: Store MEK in Stronghold: VAULT_COMMUNITIES / mek_{community_id}
    // TODO: Distribute MEK to members via their individual Signal sessions
    drop(mek);

    state
        .communities
        .write()
        .insert(community_id.clone(), community);

    tracing::info!(community = %community_id, name = %name, "community created (local only)");
    Ok(community_id)
}

/// Join an existing community by ID or invite code.
pub async fn join_community(
    state: &Arc<AppState>,
    community_id: &str,
) -> Result<(), String> {
    // Clone routing context out of the parking_lot lock before any .await
    let routing_context = {
        let node = state.node.read();
        node.as_ref()
            .filter(|nh| nh.is_attached)
            .map(|nh| nh.routing_context.clone())
    };

    let (name, description, channels, dht_record_key) = if let Some(rc) = routing_context {
        let mgr = DHTManager::new(rc);

        // Try to open the community's DHT record (community_id may be the DHT key itself)
        match mgr.open_record(community_id).await {
            Ok(()) => {
                // Read metadata from subkey 0
                let (name, description) =
                    match mgr.get_value(community_id, SUBKEY_METADATA).await {
                        Ok(Some(data)) => parse_community_metadata(&data, community_id),
                        Ok(None) => (default_community_name(community_id), None),
                        Err(e) => {
                            tracing::warn!(error = %e, "failed to read community metadata from DHT");
                            (default_community_name(community_id), None)
                        }
                    };

                // Read channel list from subkey 1
                let channels = match mgr.get_value(community_id, SUBKEY_CHANNELS).await {
                    Ok(Some(data)) => parse_channel_list(&data),
                    Ok(None) | Err(_) => vec![],
                };

                // Watch the community record for changes (metadata=0, channels=1, members=2)
                if let Err(e) = mgr.watch_record(community_id, &[0, 1, 2]).await {
                    tracing::warn!(error = %e, "failed to watch community DHT record");
                }

                (name, description, channels, Some(community_id.to_string()))
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to open community DHT record, joining locally");
                (default_community_name(community_id), None, vec![], None)
            }
        }
    } else {
        (default_community_name(community_id), None, vec![], None)
    };

    // TODO: Receive MEK for the community (distributed via Signal session)

    // Read our identity outside the parking_lot lock before any .await
    let (our_public_key, our_display_name) = {
        let identity = state.identity.read();
        match identity.as_ref() {
            Some(id) => (id.public_key.clone(), id.display_name.clone()),
            None => (String::new(), String::new()),
        }
    };

    // Update DHT member list (subkey 2) to include ourselves
    if let Some(ref dht_key) = dht_record_key {
        add_self_to_dht_members(state, dht_key, &our_public_key, &our_display_name).await;
    }

    let community = CommunityState {
        id: community_id.to_string(),
        name,
        description,
        channels,
        my_role: Some("member".to_string()),
        dht_record_key,
    };

    state
        .communities
        .write()
        .insert(community_id.to_string(), community);

    tracing::info!(community = %community_id, "joined community");
    Ok(())
}

/// Add ourselves to a community's DHT member list (subkey 2) if not already present.
async fn add_self_to_dht_members(
    state: &Arc<AppState>,
    dht_key: &str,
    our_public_key: &str,
    our_display_name: &str,
) {
    let routing_context = {
        let node = state.node.read();
        node.as_ref()
            .filter(|nh| nh.is_attached)
            .map(|nh| nh.routing_context.clone())
    };

    let Some(rc) = routing_context else { return };
    let mgr = DHTManager::new(rc);

    // Read existing member list from subkey 2
    let mut members: Vec<serde_json::Value> =
        match mgr.get_value(dht_key, SUBKEY_MEMBERS).await {
            Ok(Some(data)) => serde_json::from_slice(&data).unwrap_or_default(),
            Ok(None) | Err(_) => Vec::new(),
        };

    // Check if we're already in the list
    let already_member = members.iter().any(|m| {
        m.get("publicKey")
            .and_then(|v| v.as_str())
            .is_some_and(|k| k == our_public_key)
    });

    if already_member {
        return;
    }

    members.push(serde_json::json!({
        "publicKey": our_public_key,
        "displayName": our_display_name,
        "role": "member",
    }));

    match serde_json::to_vec(&members) {
        Ok(members_bytes) => {
            if let Err(e) = mgr.set_value(dht_key, SUBKEY_MEMBERS, members_bytes).await {
                tracing::warn!(error = %e, "failed to update member list in DHT");
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "failed to serialize member list");
        }
    }
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
    // Role-based access check and collect current channels + DHT key
    let (existing_channels, dht_record_key) = {
        let communities = state.communities.read();
        let community = communities
            .get(community_id)
            .ok_or_else(|| format!("community {community_id} not found"))?;

        let role = community.my_role.as_deref().unwrap_or("member");
        if !ROLE_CAN_CREATE_CHANNEL.contains(&role) {
            return Err(format!(
                "insufficient permissions: role '{role}' cannot create channels"
            ));
        }

        (community.channels.clone(), community.dht_record_key.clone())
    };

    let channel_id = format!("channel_{}", hex::encode(rand_bytes(8)));

    let ch_type = match channel_type {
        "voice" => ChannelType::Voice,
        _ => ChannelType::Text,
    };

    let channel = ChannelInfo {
        id: channel_id.clone(),
        name: channel_name.to_string(),
        channel_type: ch_type,
        unread_count: 0,
    };

    // Add to community state
    {
        let mut communities = state.communities.write();
        if let Some(community) = communities.get_mut(community_id) {
            community.channels.push(channel);
        } else {
            return Err(format!("community {community_id} not found"));
        }
    }

    // Update community DHT record subkey 1 (channel list)
    if let Some(dht_key) = &dht_record_key {
        let routing_context = {
            let node = state.node.read();
            node.as_ref()
                .filter(|nh| nh.is_attached)
                .map(|nh| nh.routing_context.clone())
        };

        if let Some(rc) = routing_context {
            let mgr = DHTManager::new(rc);

            // Build updated channel list including the new channel
            let mut all_channels = existing_channels;
            all_channels.push(ChannelInfo {
                id: channel_id.clone(),
                name: channel_name.to_string(),
                channel_type: match channel_type {
                    "voice" => ChannelType::Voice,
                    _ => ChannelType::Text,
                },
                unread_count: 0,
            });

            let channels_json: Vec<serde_json::Value> = all_channels
                .iter()
                .map(|ch| {
                    serde_json::json!({
                        "id": ch.id,
                        "name": ch.name,
                        "channelType": match ch.channel_type {
                            ChannelType::Text => "text",
                            ChannelType::Voice => "voice",
                        },
                    })
                })
                .collect();

            let channels_bytes = serde_json::to_vec(&channels_json)
                .map_err(|e| format!("failed to serialize channels: {e}"))?;

            if let Err(e) = mgr.set_value(dht_key, SUBKEY_CHANNELS, channels_bytes).await {
                tracing::warn!(error = %e, "failed to update channel list in DHT");
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

/// Rotate the MEK for a community (e.g., when a member is removed).
pub fn rotate_mek(community_id: &str, new_generation: u64) -> MediaEncryptionKey {
    let mek = MediaEncryptionKey::generate(new_generation);
    tracing::info!(
        community = %community_id,
        generation = new_generation,
        "MEK rotated"
    );
    // TODO: Store new MEK in Stronghold
    // TODO: Distribute new MEK to remaining members via Signal sessions
    mek
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
fn parse_channel_list(data: &[u8]) -> Vec<ChannelInfo> {
    let Ok(channel_list) = serde_json::from_slice::<Vec<serde_json::Value>>(data) else {
        return vec![];
    };

    channel_list
        .iter()
        .filter_map(|ch| {
            let id = ch.get("id")?.as_str()?.to_string();
            let ch_name = ch.get("name")?.as_str()?.to_string();
            let ch_type = match ch.get("channelType").and_then(|v| v.as_str()) {
                Some("voice") => ChannelType::Voice,
                _ => ChannelType::Text,
            };
            Some(ChannelInfo {
                id,
                name: ch_name,
                channel_type: ch_type,
                unread_count: 0,
            })
        })
        .collect()
}

/// Construct a default community display name from a (potentially long) ID.
fn default_community_name(community_id: &str) -> String {
    format!("Community {}", &community_id[..8.min(community_id.len())])
}

fn rand_bytes(len: usize) -> Vec<u8> {
    use rand::RngCore;
    let mut bytes = vec![0u8; len];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes
}
