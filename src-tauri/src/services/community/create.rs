use std::sync::Arc;

use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_protocol::dht::community::types::{
    ChannelEntryV2, ChannelKind, CommunityMetadataV2, CoordinatorInfo, RoleEntryV2,
};
use rekindle_protocol::dht::community::{manifest, member_registry, permissions_v2};
use rekindle_protocol::dht::DHTManager;

use crate::state::{AppState, ChannelInfo, ChannelType, CommunityState, GossipOverlay, RoleDefinition};
use crate::state_helpers;

/// Create a new community and publish it to DHT.
///
/// Creates manifest + member registry DHT records and starts the coordinator service.
/// Requires the Veilid node to be attached.
pub async fn create_community(
    state: &Arc<AppState>,
    name: &str,
) -> Result<String, String> {
    let routing_context = state_helpers::routing_context(state)
        .ok_or("Veilid node not attached — cannot create community")?;

    let mgr = DHTManager::new(routing_context.clone());
    let my_pseudonym_key = derive_pseudonym_key(state, "temp_derive")
        .unwrap_or_default();
    let now_secs = rekindle_utils::timestamp_secs();

    // 1. Create manifest (DFLT 16 subkeys)
    let metadata = CommunityMetadataV2 {
        name: name.to_string(),
        description: None,
        icon_hash: None,
        created_at: now_secs,
        owner_pseudonym: my_pseudonym_key.clone(),
        last_refreshed: now_secs,
    };
    let (manifest_key, manifest_keypair) = manifest::create_manifest(&mgr, &metadata)
        .await
        .map_err(|e| format!("failed to create manifest DHT record: {e}"))?;

    // Re-derive pseudonym with the actual manifest key as community_id
    let my_pseudonym_key = derive_pseudonym_key(state, &manifest_key)
        .unwrap_or_default();

    // 2. Generate slot seed and create pre-allocated SMPL member registry (256 slots).
    //    This allows any member to write their own presence to their assigned slot.
    let slot_seed = rand_bytes(32);
    let slot_seed_array: [u8; 32] = slot_seed.clone().try_into()
        .map_err(|_| "failed to generate 32-byte slot seed")?;
    let (registry_key, registry_keypair) = member_registry::create_registry_segment(&mgr, &slot_seed_array)
        .await
        .map_err(|e| format!("failed to create pre-allocated member registry: {e}"))?;
    let registry_owner_kp_str = registry_keypair.as_ref().map(std::string::ToString::to_string);

    // Derive creator's slot keypair (slot 0) for writing presence
    let creator_slot_keypair = member_registry::derive_slot_veilid_keypair(&slot_seed_array, 0)
        .map_err(|e| format!("failed to derive creator slot keypair: {e}"))?;
    let creator_slot_keypair_str = creator_slot_keypair.to_string();

    // 3. Write initial roles to manifest
    let roles = default_roles_for_manifest();
    if let Err(e) = manifest::write_roles(&mgr, &manifest_key, &roles).await {
        tracing::warn!(error = %e, "failed to write initial roles to manifest");
    }

    // 4. Create SMPL channel record for the default "general" channel
    let channel_id = format!("channel_{}", hex::encode(rand_bytes(8)));
    let (channel_record_key, _channel_record_owner_kp) =
        rekindle_protocol::dht::community::channel_record::create_smpl_channel_record(
            &mgr, &slot_seed_array,
        )
        .await
        .map_err(|e| format!("failed to create SMPL channel record: {e}"))?;
    tracing::debug!(channel = %channel_id, record_key = %channel_record_key, "created SMPL channel record");

    let channel_entry = ChannelEntryV2 {
        id: channel_id.clone(),
        name: "general".to_string(),
        kind: ChannelKind::Text,
        sort_order: 0,
        category_id: None,
        topic: String::new(),
        slowmode_seconds: 0,
        nsfw: false,
        message_record_key: None,
        mek_generation: 0,
        permission_overwrites: Vec::new(),
        log_key: Some(channel_record_key.clone()),
    };
    if let Err(e) = manifest::write_channels(&mgr, &manifest_key, &[channel_entry]).await {
        tracing::warn!(error = %e, "failed to write initial channels to manifest");
    }

    // 5. Write coordinator info (we are the first coordinator)
    let our_route_blob = state_helpers::our_route_blob(state).unwrap_or_default();
    let coordinator_info = CoordinatorInfo {
        pseudonym_key: my_pseudonym_key.clone(),
        route_blob: our_route_blob.clone(),
        epoch: 1,
        capabilities: Vec::new(),
        heartbeat_at: now_secs,
    };
    if let Err(e) = manifest::write_coordinator(&mgr, &manifest_key, &coordinator_info).await {
        tracing::warn!(error = %e, "failed to write coordinator info to manifest");
    }

    // 6. Write initial owner to member registry
    let owner_member = rekindle_protocol::dht::community::types::MemberSummary {
        pseudonym_key: my_pseudonym_key.clone(),
        display_name: state_helpers::identity_display_name(state),
        role_ids: vec![0, 1, 2, 3, 4],
        // Use 0 so the creator is immediately eligible for coordinator election.
        // The MIN_JOIN_AGE_SECS check prevents join-and-takeover, but the founding
        // member can't "takeover" their own community.
        joined_at: 0,
        subkey_index: 0,
        onboarding_complete: true,
        timeout_until: None,
    };
    if let Err(e) = member_registry::write_member_index(&mgr, &registry_key, &[owner_member]).await {
        tracing::warn!(error = %e, "failed to write owner to member registry");
    }

    // 6b. Write registry spine to manifest subkey 12 so joiners can discover the registry key
    let spine = member_registry::single_segment_spine(&registry_key, Vec::new(), 1);
    if let Err(e) = member_registry::write_registry_spine(&mgr, &manifest_key, &spine).await {
        tracing::warn!(error = %e, "failed to write registry spine to manifest");
    }

    // 7. Generate MEK
    let mek = MediaEncryptionKey::generate(1);
    let mek_generation = mek.generation();
    state.mek_cache.lock().insert(manifest_key.clone(), mek);
    tracing::debug!(community = %manifest_key, mek_generation, "generated initial MEK for community");

    // 8. Build CommunityState
    let default_channel = ChannelInfo {
        id: channel_id.clone(),
        name: "general".to_string(),
        channel_type: ChannelType::Text,
        unread_count: 0,
        category_id: None,
        topic: String::new(),
        slowmode_seconds: None,
        nsfw: false,
        message_record_key: None,
        mek_generation: 0,
    };
    let dht_owner_keypair = manifest_keypair.as_ref().map(std::string::ToString::to_string);

    let community = CommunityState {
        id: manifest_key.clone(),
        name: name.to_string(),
        description: None,
        channels: vec![default_channel],
        categories: Vec::new(),
        my_role_ids: vec![0, 1, 2, 3, 4],
        roles: roles_to_definitions(&roles),
        my_role: Some("owner".to_string()),
        dht_record_key: Some(manifest_key.clone()),
        dht_owner_keypair,
        my_pseudonym_key: Some(my_pseudonym_key.clone()),
        mek_generation,
        manifest_key: Some(manifest_key.clone()),
        member_registry_key: Some(registry_key),
        my_subkey_index: Some(0),
        coordinator_pseudonym: Some(my_pseudonym_key.clone()),
        coordinator_route_blob: Some(our_route_blob),
        coordinator_epoch: 1,
        gossip: Some(GossipOverlay::default()),
        slot_keypair: Some(creator_slot_keypair_str.clone()),
        manifest_owner_keypair: manifest_keypair.as_ref().map(std::string::ToString::to_string),
        channel_log_keys: [(channel_id, channel_record_key)].into_iter().collect(),
        channel_sequences: std::collections::HashMap::new(),
        pending_syncs: std::collections::HashMap::new(),
        registry_owner_keypair: registry_owner_kp_str,
        slot_seed: Some(hex::encode(&slot_seed)),
        member_roles: std::collections::HashMap::new(),
        known_members: [my_pseudonym_key].into_iter().collect(),
        presence_poll_shutdown_tx: None,
        dht_keepalive_shutdown_tx: None,
    };

    state.communities.write().insert(manifest_key.clone(), community);

    // 9. Create coordinator handle (static owner model — creator is permanent coordinator).
    let handle = crate::services::coordinator::create_handle(state, manifest_key.clone());
    *handle.role.write() = crate::services::coordinator::CoordinatorRole::Coordinator;
    state
        .coordinator_services
        .write()
        .insert(manifest_key.clone(), handle);

    // 10. Start presence poll and DHT keepalive
    super::presence::start_presence_poll(state.clone(), manifest_key.clone());
    super::keepalive::start_dht_keepalive(state.clone(), manifest_key.clone());

    tracing::info!(name = %name, manifest_key = %manifest_key, "community created with DHT records");
    Ok(manifest_key)
}

/// Derive the pseudonym public key hex for a community from the identity secret.
pub(crate) fn derive_pseudonym_key(state: &Arc<AppState>, community_id: &str) -> Option<String> {
    let secret = state.identity_secret.lock();
    secret.as_ref().map(|s| {
        let signing_key =
            rekindle_crypto::group::pseudonym::derive_community_pseudonym(s, community_id);
        hex::encode(signing_key.verifying_key().to_bytes())
    })
}

/// Default role definitions for a newly created community (DHT manifest format).
pub(crate) fn default_roles_for_manifest() -> Vec<RoleEntryV2> {
    vec![
        RoleEntryV2 {
            id: 0,
            name: "@everyone".to_string(),
            color: 0,
            permissions: permissions_v2::everyone_default().bits(),
            position: 0,
            hoist: false,
            mentionable: false,
        },
        RoleEntryV2 {
            id: 1,
            name: "Members".to_string(),
            color: 0,
            permissions: permissions_v2::member_default().bits(),
            position: 1,
            hoist: false,
            mentionable: false,
        },
        RoleEntryV2 {
            id: 2,
            name: "Moderator".to_string(),
            color: 0x0034_98DB,
            permissions: permissions_v2::moderator_default().bits(),
            position: 2,
            hoist: true,
            mentionable: true,
        },
        RoleEntryV2 {
            id: 3,
            name: "Admin".to_string(),
            color: 0x00E7_4C3C,
            permissions: permissions_v2::admin_default().bits(),
            position: 3,
            hoist: true,
            mentionable: true,
        },
        RoleEntryV2 {
            id: 4,
            name: "Owner".to_string(),
            color: 0x00F1_C40F,
            permissions: permissions_v2::owner_default().bits(),
            position: 4,
            hoist: true,
            mentionable: false,
        },
    ]
}

/// Convert DHT role entries to RoleDefinition for CommunityState in-memory use.
pub(crate) fn roles_to_definitions(roles: &[RoleEntryV2]) -> Vec<RoleDefinition> {
    roles
        .iter()
        .map(|r| RoleDefinition {
            id: r.id,
            name: r.name.clone(),
            color: r.color,
            permissions: r.permissions,
            position: r.position,
            hoist: r.hoist,
            mentionable: r.mentionable,
        })
        .collect()
}

fn rand_bytes(len: usize) -> Vec<u8> {
    use rand::RngCore;
    let mut bytes = vec![0u8; len];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes
}
