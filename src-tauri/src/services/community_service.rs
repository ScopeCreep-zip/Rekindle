use std::sync::Arc;

use tauri::Manager as _;

use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_protocol::dht::community::{manifest, member_registry, permissions_v2};
use rekindle_protocol::dht::community::types::{
    ChannelEntryV2, ChannelKind, CommunityMetadataV2, CoordinatorInfo, MemberSummary, RoleEntryV2,
};
use rekindle_protocol::dht::DHTManager;
use crate::state::{AppState, CategoryInfo, ChannelInfo, ChannelType, CommunityState, GossipOverlay, RoleDefinition};
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

    // 4. Create a DHTLog for the default "general" channel, then write to manifest
    let channel_id = format!("channel_{}", hex::encode(rand_bytes(8)));
    let (channel_log_key, channel_log_keypair) = rekindle_protocol::dht::community::channel_record::create_channel_log(
        &routing_context,
    )
    .await
    .map_err(|e| format!("failed to create channel DHTLog: {e}"))?;
    tracing::debug!(channel = %channel_id, log_key = %channel_log_key, "created channel DHTLog");

    // Persist channel log keypair to Stronghold
    {
        use tauri::Manager as _;
        let app_handle = state.app_handle.read().clone();
        if let Some(ref ah) = app_handle {
            let ks_handle: tauri::State<'_, crate::keystore::KeystoreHandle> = ah.state();
            let ks = ks_handle.lock();
            if let Some(ref keystore) = *ks {
                crate::keystore::persist_channel_log_keypair(
                    keystore, &manifest_key, &channel_id, &channel_log_keypair.to_string(),
                );
            }
        }
    }

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
        log_key: Some(channel_log_key.clone()),
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
    let owner_member = MemberSummary {
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
        channel_log_keys: [(channel_id, channel_log_key)].into_iter().collect(),
        registry_owner_keypair: registry_owner_kp_str,
        slot_seed: Some(hex::encode(&slot_seed)),
        known_members: [my_pseudonym_key].into_iter().collect(),
        presence_poll_shutdown_tx: None,
        dht_keepalive_shutdown_tx: None,
    };

    state.communities.write().insert(manifest_key.clone(), community);

    // 9. Start coordinator service (we are the first coordinator).
    // Set role to Coordinator immediately — don't wait for the 5s election timer.
    let handle = super::coordinator::start(state.clone(), manifest_key.clone());
    *handle.role.write() = super::coordinator::CoordinatorRole::Coordinator;
    state
        .coordinator_services
        .write()
        .insert(manifest_key.clone(), handle);

    // 10. Start presence poll and DHT keepalive
    start_presence_poll(state.clone(), manifest_key.clone());
    start_dht_keepalive(state.clone(), manifest_key.clone());

    tracing::info!(name = %name, manifest_key = %manifest_key, "community created with DHT records");
    Ok(manifest_key)
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

/// Join a community via self-service SMPL presence registration.
///
/// Zero coordinator dependency. The invite code decrypts embedded secrets
/// (slot_seed, MEK, subkey_index, registry_key) from the DHT manifest.
/// The joiner derives their slot keypair, writes presence to the SMPL
/// registry (proof of membership), and starts the gossip mesh.
///
/// Flow: Read manifest → decrypt invite → derive slot keypair → write
/// presence → start services. No MemberJoinRequest, no JoinAccepted.
pub async fn join_community(
    state: &Arc<AppState>,
    community_id: &str,
    invite_code: Option<&str>,
) -> Result<(), String> {
    let invite_code =
        invite_code.ok_or("invite code required — community join requires a valid invite link")?;

    let rc = state_helpers::routing_context(state)
        .ok_or("Veilid node not attached — cannot join community")?;
    let mgr = DHTManager::new(rc.clone());

    // ── BOOTSTRAP: DHT reads (no coordinator, no online members needed) ──

    // 1. Open and read manifest from DHT
    mgr.open_record(community_id)
        .await
        .map_err(|e| format!("failed to open manifest record: {e}"))?;

    let metadata = manifest::read_metadata(&mgr, community_id)
        .await
        .map_err(|e| format!("failed to read metadata: {e}"))?;
    let name = metadata.as_ref().map_or_else(
        || default_community_name(community_id),
        |m| m.name.clone(),
    );
    let description = metadata.as_ref().and_then(|m| m.description.clone());

    let channel_entries = manifest::read_channels(&mgr, community_id)
        .await
        .unwrap_or_default();
    let category_entries = manifest::read_categories(&mgr, community_id)
        .await
        .unwrap_or_default();
    let role_entries = manifest::read_roles(&mgr, community_id)
        .await
        .unwrap_or_default();

    // Coordinator info is optional for self-service join — community works without it
    let coordinator = manifest::read_coordinator(&mgr, community_id)
        .await
        .ok()
        .flatten();

    // Watch manifest for changes
    if let Err(e) = manifest::watch_manifest(&mgr, community_id).await {
        tracing::warn!(error = %e, "failed to watch manifest");
    }

    // 2. Read invites from manifest → find ours by code hash → decrypt secrets
    let code_hash = rekindle_crypto::group::invite_crypto::hash_invite_code(invite_code);
    let invites = manifest::read_invites(&mgr, community_id)
        .await
        .map_err(|e| format!("failed to read invites: {e}"))?;

    let invite_entry = invites
        .iter()
        .find(|inv| inv.code_hash == code_hash)
        .ok_or("invalid invite code")?;

    // Validate expiry
    let now = rekindle_utils::timestamp_secs();
    if let Some(expires_at) = invite_entry.expires_at {
        if now > expires_at {
            return Err("invite has expired".into());
        }
    }

    // Validate max uses (approximate — slot occupation is the real limit)
    if invite_entry.max_uses > 0 && invite_entry.use_count >= invite_entry.max_uses {
        return Err("invite has reached maximum uses".into());
    }

    let encrypted_b64 = invite_entry
        .encrypted_secrets
        .as_ref()
        .ok_or("invite does not contain embedded secrets")?;

    let encrypted = {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .decode(encrypted_b64)
            .map_err(|e| format!("invalid invite secrets encoding: {e}"))?
    };
    let secrets_json =
        rekindle_crypto::group::invite_crypto::decrypt_invite_secrets(invite_code, &encrypted)
            .map_err(|e| format!("failed to decrypt invite secrets: {e}"))?;
    let secrets: rekindle_protocol::dht::community::InviteSecrets =
        serde_json::from_slice(&secrets_json)
            .map_err(|e| format!("invalid invite secrets format: {e}"))?;

    // 3. Extract secrets
    let registry_key = secrets.registry_key.clone();
    let slot_seed_hex = secrets.slot_seed.clone();

    let mek = {
        use base64::Engine;
        let mek_wire = base64::engine::general_purpose::STANDARD
            .decode(&secrets.mek_wire_bytes)
            .map_err(|e| format!("invalid MEK encoding: {e}"))?;
        MediaEncryptionKey::from_wire_bytes(&mek_wire)
            .ok_or("invalid MEK wire bytes")?
    };
    let mek_generation = mek.generation();

    // Determine subkey index (single-use or first free in range)
    let my_subkey_index = if let Some(idx) = secrets.assigned_subkey_index {
        idx
    } else if let Some((start, end)) = secrets.slot_range {
        // Multi-use: scan for first empty slot in range, write presence,
        // then re-read to verify we won the slot (collision retry).
        mgr.open_record(&registry_key)
            .await
            .map_err(|e| format!("failed to open registry: {e}"))?;

        let members = member_registry::read_member_index(&mgr, &registry_key)
            .await
            .unwrap_or_default();
        let occupied: std::collections::HashSet<u32> =
            members.iter().map(|m| m.subkey_index).collect();

        let my_pk = derive_pseudonym_key(state, community_id);
        let slot_seed_bytes_tmp = hex::decode(&slot_seed_hex)
            .map_err(|e| format!("invalid slot seed hex: {e}"))?;
        let slot_seed_arr: [u8; 32] = slot_seed_bytes_tmp
            .as_slice()
            .try_into()
            .map_err(|_| "slot seed must be 32 bytes")?;

        let mut claimed_slot = None;
        for candidate in (start..=end).filter(|idx| !occupied.contains(idx)) {
            // Derive keypair for this candidate slot
            let kp = member_registry::derive_slot_veilid_keypair(&slot_seed_arr, candidate)
                .map_err(|e| format!("derive slot keypair: {e}"))?;

            // Write presence to claim the slot
            let presence = rekindle_protocol::dht::community::types::MemberPresence {
                pseudonym_key: my_pk.clone().unwrap_or_default(),
                status: "online".to_string(),
                status_message: None,
                game_info: None,
                route_blob: None, // Updated by presence_poll_tick
                last_heartbeat: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
                is_coordinator: false,
                coordinator_since: 0,
                is_archiver: false,
            };
            if member_registry::write_member_presence(&mgr, &registry_key, candidate, &presence, kp)
                .await
                .is_err()
            {
                continue; // Slot write failed, try next
            }

            // Re-read to verify we own the slot (collision detection)
            match member_registry::read_member_presence(&mgr, &registry_key, candidate).await {
                Ok(Some(readback)) if readback.pseudonym_key == presence.pseudonym_key => {
                    claimed_slot = Some(candidate);
                    break;
                }
                _ => {
                    tracing::debug!(slot = candidate, "multi-use invite slot collision, trying next");
                }
            }
        }

        claimed_slot.ok_or("all slots in invite range are occupied or contested")?
    } else {
        return Err("invite secrets contain no subkey_index or slot_range".into());
    };

    // 4. Derive our pseudonym key
    let my_pseudonym_key = derive_pseudonym_key(state, community_id);

    // 5. Convert DHT entries to in-memory types
    let channels: Vec<ChannelInfo> = channel_entries
        .iter()
        .map(|ch| ChannelInfo {
            id: ch.id.clone(),
            name: ch.name.clone(),
            channel_type: ch.kind.as_str().parse().unwrap_or(ChannelType::Text),
            unread_count: 0,
            category_id: ch.category_id.clone(),
            topic: ch.topic.clone(),
            slowmode_seconds: if ch.slowmode_seconds > 0 { Some(ch.slowmode_seconds) } else { None },
            nsfw: ch.nsfw,
            message_record_key: ch.message_record_key.clone(),
            mek_generation: ch.mek_generation,
        })
        .collect();
    let categories: Vec<CategoryInfo> = category_entries
        .iter()
        .map(|cat| CategoryInfo {
            id: cat.id.clone(),
            name: cat.name.clone(),
            sort_order: cat.sort_order,
        })
        .collect();
    let roles = if role_entries.is_empty() {
        roles_to_definitions(&default_roles_for_manifest())
    } else {
        roles_to_definitions(&role_entries)
    };

    // Read registry spine for the member_registry_key (belt and suspenders — also in secrets)
    let registry_key_from_spine = match member_registry::read_registry_spine(&mgr, community_id).await {
        Ok(Some(spine)) if !spine.segments.is_empty() => {
            Some(spine.segments[0].record_key.clone())
        }
        _ => None,
    };
    let final_registry_key = registry_key_from_spine.unwrap_or(registry_key.clone());

    // ── REGISTER: derive slot keypair ──

    // 6. Derive slot keypair from seed + index
    let slot_seed_bytes = hex::decode(&slot_seed_hex)
        .map_err(|e| format!("invalid slot seed hex: {e}"))?;
    let slot_seed_array: [u8; 32] = slot_seed_bytes
        .as_slice()
        .try_into()
        .map_err(|_| "slot seed must be 32 bytes")?;

    let slot_keypair = member_registry::derive_slot_veilid_keypair(&slot_seed_array, my_subkey_index)
        .map_err(|e| format!("failed to derive slot keypair: {e}"))?;
    let slot_keypair_str = slot_keypair.to_string();

    // 7. Cache MEK
    state.mek_cache.lock().insert(community_id.to_string(), mek);

    // 7b. Read member index for known_members seeding (so we accept messages from existing members)
    let existing_members = member_registry::read_member_index(&mgr, &final_registry_key)
        .await
        .unwrap_or_default();

    // 8. Build CommunityState with everything from DHT + decrypted secrets
    let community = CommunityState {
        id: community_id.to_string(),
        name,
        description,
        channels,
        categories,
        my_role_ids: vec![0, 1], // Default roles, updated when admin adds to member index
        roles,
        my_role: Some("member".to_string()),
        dht_record_key: Some(community_id.to_string()),
        dht_owner_keypair: None,
        my_pseudonym_key: my_pseudonym_key.clone(),
        mek_generation,
        manifest_key: Some(community_id.to_string()),
        member_registry_key: Some(final_registry_key),
        my_subkey_index: Some(my_subkey_index),
        coordinator_pseudonym: coordinator.as_ref().map(|c| c.pseudonym_key.clone()),
        coordinator_route_blob: coordinator.as_ref().map(|c| c.route_blob.clone()),
        coordinator_epoch: coordinator.as_ref().map_or(0, |c| c.epoch),
        gossip: None,
        slot_keypair: Some(slot_keypair_str),
        manifest_owner_keypair: None,
        channel_log_keys: std::collections::HashMap::new(),
        registry_owner_keypair: None,
        slot_seed: Some(slot_seed_hex),
        known_members: existing_members.iter().map(|m| m.pseudonym_key.clone()).collect(), // Seed from DHT registry
        presence_poll_shutdown_tx: None,
        dht_keepalive_shutdown_tx: None,
    };

    state.communities.write().insert(community_id.to_string(), community);

    // Persist discovered members to SQLite so get_community_members works immediately
    sync_members_to_state_and_db(state, community_id, &existing_members);

    // ── CONNECT: start services ──

    // 9. Start coordinator service (election watcher — useful but not required)
    let handle = super::coordinator::start(state.clone(), community_id.to_string());
    state.coordinator_services.write().insert(community_id.to_string(), handle);

    // 10. Start presence poll (IMMEDIATE first tick writes our presence + discovers peers)
    //     and DHT keepalive. Presence write is our proof of membership — Veilid validates
    //     the slot keypair against the SMPL schema.
    start_presence_poll(state.clone(), community_id.to_string());
    start_dht_keepalive(state.clone(), community_id.to_string());

    tracing::info!(
        community = %community_id,
        subkey_index = my_subkey_index,
        "self-service join complete — presence poll will write to SMPL registry"
    );

    // 11. Broadcast MemberJoinRequest via gossip so any online admin can add us
    //     to the member index. Delayed slightly to allow presence_poll_tick to
    //     discover peers first (gossip overlay needs at least one peer).
    {
        let state = state.clone();
        let cid = community_id.to_string();
        let display_name = {
            let identity = state.identity.read();
            identity.as_ref().map_or_else(
                || "Unknown".to_string(),
                |id| id.display_name.clone(),
            )
        };
        let pk = my_pseudonym_key.unwrap_or_default();
        let idx = my_subkey_index;
        let our_route = state_helpers::our_route_blob(&state);
        tokio::spawn(async move {
            // Wait for presence poll to build gossip overlay
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            let envelope = rekindle_protocol::dht::community::envelope::CommunityEnvelope::Control(
                rekindle_protocol::dht::community::envelope::ControlPayload::MemberJoinRequest {
                    pseudonym_key: pk,
                    display_name,
                    invite_code: None,
                    route_blob: our_route,
                    prekey_bundle: None,
                    claimed_subkey_index: Some(idx),
                },
            );
            if let Err(e) = crate::commands::community::send_to_mesh(&state, &cid, &envelope) {
                tracing::debug!(
                    community = %cid,
                    error = %e,
                    "failed to broadcast MemberJoinRequest — admin will discover via presence scan"
                );
            }
        });
    }

    Ok(())
}

/// Construct a default community display name from a (potentially long) ID.
fn default_community_name(community_id: &str) -> String {
    format!("Community {}", &community_id[..8.min(community_id.len())])
}

/// Default role definitions for a newly created community (DHT manifest format).
fn default_roles_for_manifest() -> Vec<RoleEntryV2> {
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
fn roles_to_definitions(roles: &[RoleEntryV2]) -> Vec<RoleDefinition> {
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

/// Re-announce our route to the community after restart via gossip mesh.
///
/// Broadcasts a `PresenceUpdate` via gossip so all peers learn our fresh route.
/// Also tries to re-fetch the coordinator route from DHT manifest (needed only
/// for the few truly coordinator-dependent ops: join processing, MEK distribution).
pub async fn rejoin_community(state: &Arc<AppState>, community_id: &str) -> Result<(), String> {
    if crate::state_helpers::is_circuit_open(state, community_id) {
        tracing::debug!(community = %community_id, "skipping rejoin — circuit breaker open");
        return Ok(());
    }

    // The coordinator doesn't need to rejoin their own community — they already
    // have full state. Sending MemberJoinRequest to ourselves triggers a
    // self-JoinAccepted that can corrupt role_ids.
    let is_coordinator = {
        let services = state.coordinator_services.read();
        services
            .get(community_id)
            .is_some_and(super::coordinator::CoordinatorServiceHandle::is_coordinator)
    };
    if is_coordinator {
        tracing::debug!(community = %community_id, "skipping rejoin — we are the coordinator");
        return Ok(());
    }

    let coordinator_route_blob = {
        let communities = state.communities.read();
        let c = communities.get(community_id).ok_or("community not found")?;
        c.coordinator_route_blob.clone()
    };

    // If no coordinator route, try to re-fetch from DHT manifest
    if coordinator_route_blob.is_none() {
        let Some(rc) = state_helpers::routing_context(state) else { return Ok(()) };
        let manifest_key = {
            let communities = state.communities.read();
            communities.get(community_id).and_then(|c| c.manifest_key.clone())
        };
        let Some(ref key) = manifest_key else { return Ok(()) };
        let mgr = DHTManager::new(rc);
        // Open manifest before reading — may be closed after app restart
        if let Err(e) = mgr.open_record(key).await {
            tracing::warn!(community = %community_id, error = %e, "rejoin: failed to open manifest");
            return Ok(());
        }
        match manifest::read_coordinator(&mgr, key).await {
            Ok(Some(coord_info)) if !coord_info.route_blob.is_empty() => {
                tracing::info!(community = %community_id, "re-fetched coordinator route from DHT for rejoin");
                let mut communities = state.communities.write();
                if let Some(c) = communities.get_mut(community_id) {
                    c.coordinator_route_blob = Some(coord_info.route_blob);
                    c.coordinator_pseudonym = Some(coord_info.pseudonym_key);
                    c.coordinator_epoch = coord_info.epoch;
                }
            }
            _ => {
                tracing::info!(
                    community = %community_id,
                    "no coordinator online — community operates via gossip mesh"
                );
                // Community operates fully via gossip mesh. Only truly
                // coordinator-dependent ops (join processing, MEK distribution)
                // are unavailable without a coordinator route.
                return Ok(());
            }
        }
    }

    // Broadcast route announcement via gossip mesh — all peers learn our new route.
    // No coordinator needed; presence poll will also write our route to DHT registry.
    let pseudonym_key = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .and_then(|c| c.my_pseudonym_key.clone())
            .unwrap_or_default()
    };
    let our_route_blob = state_helpers::our_route_blob(state);
    let status = state_helpers::identity_status(state)
        .unwrap_or(crate::state::UserStatus::Online);
    let status_str = match status {
        crate::state::UserStatus::Online => "online",
        crate::state::UserStatus::Away => "away",
        crate::state::UserStatus::Busy => "busy",
        crate::state::UserStatus::Offline | crate::state::UserStatus::Invisible => "offline",
    };

    match crate::commands::community::send_to_mesh(
        state,
        community_id,
        &rekindle_protocol::dht::community::envelope::CommunityEnvelope::PresenceUpdate {
            pseudonym_key,
            status: status_str.to_string(),
            game_info: None,
            route_blob: our_route_blob,
        },
    ) {
        Ok(()) => {
            state_helpers::reset_circuit_breaker(state, community_id);
            tracing::debug!(community = %community_id, "re-announced route via gossip mesh");
        }
        Err(e) => {
            tracing::warn!(community = %community_id, error = %e, "rejoin gossip broadcast failed");
            state_helpers::trip_circuit_breaker(state, community_id);
        }
    }
    Ok(())
}

/// Start the 60-second presence poll loop for a community.
///
/// The poll loop:
/// 1. Writes our signed presence to the registry
/// 2. Reads all member presences to discover who is online
/// 3. Updates the gossip overlay peer set (random D peers from online members)
/// 4. Writes coordinator heartbeat if we are coordinator
/// 5. Checks coordinator liveness if we are NOT coordinator
pub fn start_presence_poll(state: Arc<AppState>, community_id: String) {
    use tokio::sync::mpsc;

    let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
    {
        let mut communities = state.communities.write();
        if let Some(cs) = communities.get_mut(&community_id) {
            cs.presence_poll_shutdown_tx = Some(shutdown_tx);
        }
    }
    tokio::spawn(async move {
        // Run an immediate first tick so gossip overlay is populated right away
        // (don't wait 60s — members need to discover peers immediately)
        if let Err(e) = presence_poll_tick(&state, &community_id).await {
            tracing::debug!(
                community = %community_id,
                error = %e,
                "initial presence poll tick failed"
            );
        }

        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        interval.tick().await; // consume immediate tick (already ran above)
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    if let Err(e) = presence_poll_tick(&state, &community_id).await {
                        tracing::debug!(
                            community = %community_id,
                            error = %e,
                            "presence poll tick failed"
                        );
                    }
                }
                _ = shutdown_rx.recv() => break,
            }
        }
    });
}

/// Public entry point for triggering an immediate presence poll tick.
/// Called after SlotKeypairGrant to speed up peer discovery.
pub async fn presence_poll_tick_public(
    state: &Arc<AppState>,
    community_id: &str,
) -> Result<(), String> {
    presence_poll_tick(state, community_id).await
}

/// Try to derive the slot keypair locally from slot_seed + subkey_index.
/// Returns the derived keypair string if successful.
pub(crate) fn try_derive_slot_keypair(
    state: &Arc<AppState>,
    community_id: &str,
    seed_hex: &str,
    subkey_idx: u32,
) -> Option<String> {
    let seed_bytes = hex::decode(seed_hex).ok()?;
    let seed_array: [u8; 32] = seed_bytes.as_slice().try_into().ok()?;
    match member_registry::derive_slot_veilid_keypair(&seed_array, subkey_idx) {
        Ok(kp) => {
            let kp_str = kp.to_string();
            {
                let mut communities = state.communities.write();
                if let Some(c) = communities.get_mut(community_id) {
                    c.slot_keypair = Some(kp_str.clone());
                }
            }
            tracing::info!(
                community = %community_id,
                subkey = subkey_idx,
                "derived slot keypair locally from seed (self-healed)"
            );
            Some(kp_str)
        }
        Err(e) => {
            tracing::warn!(error = %e, "failed to derive slot keypair from seed");
            None
        }
    }
}

/// Sync discovered members into `known_members` (in-memory) and `community_members` (SQLite).
/// Called from both `join_community` and `presence_poll_tick` to ensure members are recognized.
fn sync_members_to_state_and_db(
    state: &Arc<AppState>,
    community_id: &str,
    members: &[rekindle_protocol::dht::community::types::MemberSummary],
) {
    // Add to known_members so we accept their messages
    {
        let mut communities = state.communities.write();
        if let Some(cs) = communities.get_mut(community_id) {
            for member in members {
                cs.known_members.insert(member.pseudonym_key.clone());
            }
        }
    }

    // Persist to SQLite so get_community_members returns them
    let app_handle_clone = state.app_handle.read().clone();
    if let Some(ref ah) = app_handle_clone {
        let pool: tauri::State<'_, crate::db::DbPool> = ah.state();
        let owner_key = crate::state_helpers::current_owner_key(state).unwrap_or_default();
        for member in members {
            let ok = owner_key.clone();
            let cid = community_id.to_string();
            let pk = member.pseudonym_key.clone();
            let dn = member.display_name.clone();
            let rids = serde_json::to_string(&member.role_ids)
                .unwrap_or_else(|_| "[0,1]".to_string());
            crate::db_helpers::db_fire(
                pool.inner(),
                "persist discovered member",
                move |conn| {
                    conn.execute(
                        "INSERT INTO community_members (owner_key, community_id, pseudonym_key, display_name, role_ids, joined_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, 0) \
                         ON CONFLICT(owner_key, community_id, pseudonym_key) \
                         DO UPDATE SET display_name=excluded.display_name, role_ids=excluded.role_ids",
                        rusqlite::params![ok, cid, pk, dn, rids],
                    )?;
                    Ok(())
                },
            );
        }
    }
}

/// Single presence poll tick.
async fn presence_poll_tick(state: &Arc<AppState>, community_id: &str) -> Result<(), String> {
    let rc = state_helpers::routing_context(state).ok_or("not attached")?;
    let mgr = DHTManager::new(rc);

    // Read member registry to scan presences
    let registry_key = {
        let communities = state.communities.read();
        let c = communities.get(community_id).ok_or("community not found")?;
        c.member_registry_key.clone()
    };
    let Some(registry_key) = registry_key else {
        return Ok(()); // No registry yet (join pending)
    };

    // Ensure registry record is open (may be closed after restart).
    // Try to open writable if we have the registry owner keypair, otherwise read-only.
    // Note: veilid's open_dht_record overwrites the writer on re-open, so opening
    // read-only here would downgrade a previous writable open. Use the owner keypair
    // if available to preserve write access for coordinator operations.
    {
        let registry_kp = {
            let communities = state.communities.read();
            communities.get(community_id).and_then(|c| c.registry_owner_keypair.clone())
        };
        let opened = if let Some(ref kp_str) = registry_kp {
            if let Ok(kp) = kp_str.parse::<veilid_core::KeyPair>() {
                mgr.open_record_writable(&registry_key, kp).await.is_ok()
            } else {
                false
            }
        } else {
            false
        };
        if !opened {
            if let Err(e) = mgr.open_record(&registry_key).await {
                tracing::debug!(community = %community_id, error = %e, "presence_poll: failed to open registry");
                return Ok(());
            }
        }
    }

    // Gather our state (clone out before .await)
    let (my_pseudonym, my_subkey_index, slot_keypair_str, slot_seed_hex, is_coordinator) = {
        let communities = state.communities.read();
        let c = communities.get(community_id).ok_or("community not found")?;
        (
            c.my_pseudonym_key.clone().unwrap_or_default(),
            c.my_subkey_index,
            c.slot_keypair.clone(),
            c.slot_seed.clone(),
            c.coordinator_pseudonym.as_ref() == c.my_pseudonym_key.as_ref()
                && c.my_pseudonym_key.is_some(),
        )
    };

    // Self-heal: if we have slot_seed + my_subkey_index but no slot_keypair,
    // derive it locally. No coordinator needed.
    let slot_keypair_str = if slot_keypair_str.is_none() {
        if let (Some(ref seed_hex), Some(subkey_idx)) = (&slot_seed_hex, my_subkey_index) {
            try_derive_slot_keypair(state, community_id, seed_hex, subkey_idx)
        } else {
            None
        }
    } else {
        slot_keypair_str
    };

    // 1. WRITE our signed presence to the registry (so others can discover our route)
    if let (Some(subkey_idx), Some(ref kp_str)) = (my_subkey_index, &slot_keypair_str) {
        let our_route_blob = state_helpers::our_route_blob(state);
        let presence = rekindle_protocol::dht::community::types::MemberPresence {
            pseudonym_key: my_pseudonym.clone(),
            status: "online".to_string(),
            status_message: None,
            game_info: None,
            route_blob: our_route_blob,
            last_heartbeat: rekindle_utils::timestamp_secs(),
            is_coordinator,
            coordinator_since: 0,
            is_archiver: false,
        };
        // Parse the Veilid KeyPair from its string representation
        if let Ok(writer_kp) = kp_str.parse::<veilid_core::KeyPair>() {
            if let Err(e) = member_registry::write_member_presence(
                &mgr, &registry_key, subkey_idx, &presence, writer_kp,
            ).await {
                tracing::debug!(
                    community = %community_id,
                    subkey = subkey_idx,
                    error = %e,
                    "failed to write our presence to DHT registry"
                );
            } else {
                tracing::trace!(
                    community = %community_id,
                    subkey = subkey_idx,
                    "wrote presence to DHT registry"
                );
            }
        } else {
            tracing::warn!(
                community = %community_id,
                "failed to parse slot keypair — cannot write presence"
            );
        }
    } else {
        // If we still don't have slot_keypair at this point, we're missing either:
        // - slot_seed (never received JoinAccepted with it, or older coordinator)
        // - my_subkey_index (never assigned a slot in the registry)
        // Try to read my_subkey_index from the DHT member registry as a last resort.
        if my_subkey_index.is_none() && slot_seed_hex.is_some() {
            let members_result = member_registry::read_member_index(&mgr, &registry_key).await;
            if let Ok(members) = members_result {
                if let Some(m) = members.iter().find(|m| m.pseudonym_key == my_pseudonym) {
                    let idx = m.subkey_index;
                    let mut communities = state.communities.write();
                    if let Some(c) = communities.get_mut(community_id) {
                        c.my_subkey_index = Some(idx);
                    }
                    tracing::info!(
                        community = %community_id,
                        subkey = idx,
                        "recovered my_subkey_index from DHT registry — will derive slot keypair next tick"
                    );
                }
            }
        }
        tracing::warn!(
            community = %community_id,
            has_slot_keypair = slot_keypair_str.is_some(),
            has_subkey_index = my_subkey_index.is_some(),
            has_slot_seed = slot_seed_hex.is_some(),
            "cannot write presence — missing slot keypair or subkey index"
        );
    }

    // 2. Read all member entries
    let members = member_registry::read_member_index(&mgr, &registry_key)
        .await
        .map_err(|e| format!("read member index: {e}"))?;

    // Sync all indexed members to known_members + SQLite
    sync_members_to_state_and_db(state, community_id, &members);

    let now_secs = rekindle_utils::timestamp_secs();
    let stale_threshold = now_secs.saturating_sub(300); // 5 minutes

    // Scan presences — build online members map
    let mut online_members: std::collections::HashMap<String, Vec<u8>> =
        std::collections::HashMap::new();
    for member in &members {
        if member.pseudonym_key == my_pseudonym {
            continue; // skip ourselves
        }
        // Read presence from member's registry subkey
        match member_registry::read_member_presence(&mgr, &registry_key, member.subkey_index).await
        {
            Ok(Some(presence)) => {
                if presence.status != "offline"
                    && presence.last_heartbeat > stale_threshold
                {
                    if let Some(blob) = presence.route_blob {
                        if !blob.is_empty() {
                            online_members.insert(member.pseudonym_key.clone(), blob);
                        }
                    }
                }
            }
            Ok(None) => {} // No presence written yet
            Err(e) => {
                tracing::trace!(
                    member = %member.pseudonym_key,
                    error = %e,
                    "failed to read member presence"
                );
            }
        }
    }

    // 3. Discover unindexed members (self-registered joiners not yet in member index)
    discover_unindexed_members(
        state, community_id, &mgr, &registry_key,
        &members, &my_pseudonym, stale_threshold, &mut online_members,
    ).await;

    // Select D random gossip peers
    let n = online_members.len();
    let d = crate::state::gossip_degree(n);
    let selected = random_peer_sample(&online_members, d);

    // Update gossip overlay
    let needs_sync = {
        let mut communities = state.communities.write();
        if let Some(cs) = communities.get_mut(community_id) {
            let counter = cs.gossip.as_ref().map_or(0, |g| g.lamport_counter);
            let was_needs_sync = cs.gossip.as_ref().is_none_or(|g| g.needs_initial_sync);
            cs.gossip = Some(GossipOverlay {
                peers: selected,
                online_members,
                lamport_counter: counter,
                needs_initial_sync: was_needs_sync,
            });
            was_needs_sync && n > 0 // Only trigger sync if peers are online
        } else {
            false
        }
    };

    // Broadcast our presence to gossip peers so they learn our route immediately
    // (don't make them wait for their own presence_poll_tick to discover us via DHT).
    if needs_sync && d > 0 {
        let (my_pk, our_route) = {
            let communities = state.communities.read();
            let cs = communities.get(community_id);
            (
                cs.and_then(|c| c.my_pseudonym_key.clone()).unwrap_or_default(),
                state_helpers::our_route_blob(state),
            )
        };
        let presence_envelope =
            rekindle_protocol::dht::community::envelope::CommunityEnvelope::PresenceUpdate {
                pseudonym_key: my_pk,
                status: "online".to_string(),
                game_info: None,
                route_blob: our_route,
            };
        let _ = crate::commands::community::send_to_mesh(state, community_id, &presence_envelope);
    }

    // Trigger SyncRequest on first successful poll with online peers
    if needs_sync {
        // Collect all channel IDs for sync
        let all_channel_ids: Vec<String> = {
            let communities = state.communities.read();
            communities.get(community_id)
                .map(|cs| cs.channels.iter().map(|ch| ch.id.clone()).collect())
                .unwrap_or_default()
        };

        // Clone AppHandle out of the lock guard before any .await
        let app_handle_clone = state.app_handle.read().clone();
        let owner_key = crate::state_helpers::current_owner_key(state).unwrap_or_default();
        if let Some(ref app_handle) = app_handle_clone {
            let pool: tauri::State<'_, crate::db::DbPool> = app_handle.state();

            for ch_id in &all_channel_ids {
                let ok = owner_key.clone();
                let ch = ch_id.clone();
                let last_ts: i64 = crate::db_helpers::db_call(pool.inner(), move |conn| {
                    conn.query_row(
                        "SELECT COALESCE(MAX(timestamp), 0) FROM messages \
                         WHERE owner_key=? AND conversation_id=? AND conversation_type='channel'",
                        rusqlite::params![ok, ch],
                        |r| r.get(0),
                    )
                }).await.unwrap_or(0);

                let sync_req = rekindle_protocol::dht::community::envelope::CommunityEnvelope::Control(
                    rekindle_protocol::dht::community::envelope::ControlPayload::SyncRequest {
                        channel_id: ch_id.clone(),
                        since_timestamp: last_ts.cast_unsigned(),
                    },
                );
                let _ = crate::commands::community::send_to_mesh(state, community_id, &sync_req);
            }
        }

        // Also catch up from DHTLogs for channels that have persistent logs
        let channel_log_entries: Vec<(String, String)> = {
            let communities = state.communities.read();
            communities.get(community_id)
                .map(|cs| cs.channel_log_keys.iter()
                    .map(|(ch_id, log_key)| (ch_id.clone(), log_key.clone()))
                    .collect())
                .unwrap_or_default()
        };

        if !channel_log_entries.is_empty() {
            if let Some(rc) = state_helpers::routing_context(state) {
                for (ch_id, log_key) in &channel_log_entries {
                    match rekindle_protocol::dht::community::channel_record::read_channel_log_tail(
                        &rc, log_key, 50,
                    ).await {
                        Ok(messages) if !messages.is_empty() => {
                            tracing::debug!(
                                community = %community_id,
                                channel = %ch_id,
                                count = messages.len(),
                                "caught up from DHTLog tail"
                            );
                            // Merge messages into local SQLite (dedup by message_id)
                            if let Some(ref app_handle) = app_handle_clone {
                                let pool: tauri::State<'_, crate::db::DbPool> = app_handle.state();
                                let channel = ch_id.clone();
                                let ok = owner_key.clone();
                                crate::db_helpers::db_fire(pool.inner(), "dhtlog_catchup", move |conn| {
                                    for msg in &messages {
                                        let mid = msg.message_id.as_deref().unwrap_or("");
                                        // Skip if message already exists
                                        let exists: bool = conn.query_row(
                                            "SELECT EXISTS(SELECT 1 FROM messages WHERE owner_key=?1 AND message_id=?2)",
                                            rusqlite::params![ok, mid],
                                            |r| r.get(0),
                                        ).unwrap_or(false);
                                        if exists { continue; }
                                        let _ = conn.execute(
                                            "INSERT OR IGNORE INTO messages \
                                             (owner_key, conversation_id, conversation_type, sender_key, body, timestamp, message_id) \
                                             VALUES (?1, ?2, 'channel', ?3, ?4, ?5, ?6)",
                                            rusqlite::params![
                                                ok, channel, msg.sender_pseudonym,
                                                String::from_utf8_lossy(&msg.ciphertext),
                                                msg.timestamp, mid,
                                            ],
                                        );
                                    }
                                    Ok(())
                                });
                            }
                        }
                        Ok(_) => {} // No messages
                        Err(e) => {
                            tracing::debug!(
                                community = %community_id,
                                channel = %ch_id,
                                error = %e,
                                "DHTLog catch-up failed"
                            );
                        }
                    }
                }
            }
        }

        // Mark sync as done
        let mut communities = state.communities.write();
        if let Some(cs) = communities.get_mut(community_id) {
            if let Some(ref mut g) = cs.gossip {
                g.needs_initial_sync = false;
            }
        }
        tracing::info!(community = %community_id, "initial sync requests sent");
    }

    tracing::trace!(
        community = %community_id,
        online = n,
        degree = d,
        "presence poll: gossip overlay updated"
    );

    Ok(())
}

/// Scan SMPL slots beyond the member index to discover self-registered joiners.
///
/// Self-service joiners write SMPL presence to their claimed slot but may not
/// be in the member index yet. This scans a window of extra slots and adds any
/// discovered members to the gossip overlay (and to the index if we're an admin).
async fn discover_unindexed_members(
    state: &Arc<AppState>,
    community_id: &str,
    mgr: &DHTManager,
    registry_key: &str,
    members: &[MemberSummary],
    my_pseudonym: &str,
    stale_threshold: u64,
    online_members: &mut std::collections::HashMap<String, Vec<u8>>,
) {
    // Build set of indexed slot indices so we only scan unindexed slots
    let indexed_slots: std::collections::HashSet<u32> =
        members.iter().map(|m| m.subkey_index).collect();
    let indexed_keys: std::collections::HashSet<&str> =
        members.iter().map(|m| m.pseudonym_key.as_str()).collect();
    // Scan ALL 255 SMPL slots to discover unindexed presences (gaps or new joiners).
    let scan_limit = member_registry::SLOTS_PER_SEGMENT;
    let has_registry_kp = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .is_some_and(|cs| cs.registry_owner_keypair.is_some())
    };
    let app_handle = state.app_handle.read().clone();

    for slot in 1..=scan_limit {
        // Skip slots that are already in the member index
        if indexed_slots.contains(&slot) {
            continue;
        }

        match member_registry::read_member_presence(mgr, registry_key, slot).await {
            Ok(Some(presence)) if !presence.pseudonym_key.is_empty() => {
                if indexed_keys.contains(presence.pseudonym_key.as_str())
                    || presence.pseudonym_key == my_pseudonym
                {
                    continue;
                }

                tracing::info!(
                    community = %community_id,
                    pseudonym = %presence.pseudonym_key,
                    slot = slot,
                    "discovered unindexed member via SMPL presence scan"
                );

                // Add to gossip overlay if online
                if presence.status != "offline" && presence.last_heartbeat > stale_threshold {
                    if let Some(blob) = presence.route_blob {
                        if !blob.is_empty() {
                            online_members.insert(presence.pseudonym_key.clone(), blob);
                        }
                    }
                }

                // Add to known_members
                {
                    let mut communities = state.communities.write();
                    if let Some(cs) = communities.get_mut(community_id) {
                        cs.known_members.insert(presence.pseudonym_key.clone());
                    }
                }

                // Emit MemberDiscovered to frontend for ALL members
                if let Some(ref app) = app_handle {
                    use tauri::Emitter as _;
                    let _ = app.emit(
                        "community-event",
                        crate::channels::CommunityEvent::MemberDiscovered {
                            community_id: community_id.to_string(),
                            pseudonym_key: presence.pseudonym_key.clone(),
                            display_name: presence.pseudonym_key.clone(),
                            subkey_index: slot,
                        },
                    );
                }

                // If admin, formally add to member index
                if has_registry_kp {
                    let display_name = presence.pseudonym_key.clone();
                    let state = state.clone();
                    let cid = community_id.to_string();
                    let pk = presence.pseudonym_key.clone();
                    tokio::spawn(async move {
                        handle_discovered_member(&state, &cid, &pk, &display_name, slot).await;
                    });
                }
            }
            _ => {
                // Empty slot or read error — continue scanning.
                // Slots may be non-contiguous (multi-use invites with slot ranges).
            }
        }
    }
}

/// Add a newly discovered (unindexed) member to the member registry and broadcast.
///
/// Called by `presence_poll_tick` when an admin discovers a SMPL presence on a slot
/// that isn't in the member index. Adds them to the index + broadcasts `MemberJoined`.
async fn handle_discovered_member(
    state: &Arc<AppState>,
    community_id: &str,
    pseudonym_key: &str,
    display_name: &str,
    claimed_slot: u32,
) {
    use rekindle_protocol::dht::community::envelope::{ControlPayload, CommunityEnvelope};

    match crate::services::coordinator::state_manager::add_member_to_registry(
        state, community_id, pseudonym_key, display_name, Some(claimed_slot),
    )
    .await
    {
        Ok(idx) => {
            tracing::info!(
                community = %community_id,
                pseudonym = %pseudonym_key,
                subkey_index = idx,
                "lazy discovery: added unindexed member to registry"
            );
            let joined = CommunityEnvelope::Control(ControlPayload::MemberJoined {
                pseudonym_key: pseudonym_key.to_string(),
                display_name: display_name.to_string(),
                role_ids: vec![0, 1],
            });
            crate::services::coordinator::state_manager::broadcast_via_gossip(
                state, community_id, &joined,
            );
            crate::services::coordinator::state_manager::emit_local_member_joined(
                state, community_id, pseudonym_key, display_name,
            );
        }
        Err(e) => {
            tracing::debug!(
                community = %community_id,
                pseudonym = %pseudonym_key,
                error = %e,
                "lazy discovery: failed to add member to registry (may already be indexed)"
            );
        }
    }
}

/// Select D random peers from the online members map.
fn random_peer_sample(
    online: &std::collections::HashMap<String, Vec<u8>>,
    d: usize,
) -> std::collections::HashMap<String, Vec<u8>> {
    use rand::seq::SliceRandom;

    if d == 0 || online.is_empty() {
        return std::collections::HashMap::new();
    }
    if d >= online.len() {
        return online.clone();
    }

    let keys: Vec<&String> = online.keys().collect();
    let mut rng = rand::rngs::OsRng;
    let selected: Vec<&String> = keys
        .choose_multiple(&mut rng, d)
        .copied()
        .collect();

    selected
        .into_iter()
        .filter_map(|k| online.get(k).map(|v| (k.clone(), v.clone())))
        .collect()
}

/// Start a DHT keepalive task that re-accesses community DHT records every 5 minutes
/// to prevent them from expiring in the Veilid DHT.
pub fn start_dht_keepalive(state: Arc<AppState>, community_id: String) {
    use tokio::sync::mpsc;

    let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
    {
        let mut communities = state.communities.write();
        if let Some(cs) = communities.get_mut(&community_id) {
            cs.dht_keepalive_shutdown_tx = Some(shutdown_tx);
        }
    }
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
        interval.tick().await; // skip immediate first tick
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    let Some(rc) = state_helpers::routing_context(&state) else {
                        continue;
                    };
                    let manifest_key = {
                        let communities = state.communities.read();
                        communities
                            .get(&community_id)
                            .and_then(|c| c.manifest_key.clone().or_else(|| Some(c.id.clone())))
                    };
                    let Some(key) = manifest_key else { continue };
                    let mgr = DHTManager::new(rc);
                    let _ = mgr.open_record(&key).await;
                    let _ = manifest::read_metadata(&mgr, &key).await;
                }
                _ = shutdown_rx.recv() => break,
            }
        }
    });
}

fn rand_bytes(len: usize) -> Vec<u8> {
    use rand::RngCore;
    let mut bytes = vec![0u8; len];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes
}
