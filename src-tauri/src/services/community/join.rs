use std::sync::Arc;

use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_protocol::dht::community::{manifest, member_registry};
use rekindle_protocol::dht::DHTManager;

use crate::state::{AppState, CategoryInfo, ChannelInfo, ChannelType, CommunityState};
use crate::state_helpers;

use super::create::{default_roles_for_manifest, derive_pseudonym_key, roles_to_definitions};

/// Join a community via self-service SMPL presence registration.
///
/// Zero coordinator dependency. The invite code decrypts embedded secrets
/// (slot_seed, MEK, subkey_index, registry_key) from the DHT manifest.
/// The joiner derives their slot keypair, writes presence to the SMPL
/// registry (proof of membership), and starts the gossip mesh.
///
/// Flow: Read manifest -> decrypt invite -> derive slot keypair -> write
/// presence -> start services. No MemberJoinRequest, no JoinAccepted.
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

    // 2. Read invites from manifest -> find ours by code hash -> decrypt secrets
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

    // Check ban list — banned members cannot rejoin
    let my_pk = derive_pseudonym_key(state, community_id);
    if let Some(ref pk) = my_pk {
        let bans = manifest::read_bans(&mgr, community_id)
            .await
            .map_err(|e| format!("failed to read ban list: {e}"))?;
        if bans.iter().any(|b| b.pseudonym_key == *pk) {
            return Err("You are banned from this community".into());
        }
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

            // Write presence to claim the slot — skip writing route_blob: None here,
            // presence_poll_tick will handle the full presence write once the route is available.
            let presence = rekindle_protocol::dht::community::types::MemberPresence {
                pseudonym_key: my_pk.clone().unwrap_or_default(),
                status: "online".to_string(),
                status_message: None,
                game_info: None,
                route_blob: None, // Updated by presence_poll_tick once route is available
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
    if final_registry_key.is_empty() {
        return Err("Failed to obtain member registry key from invite or manifest spine".into());
    }

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
        gossip: {
            // Seed provisional gossip overlay so messages arriving before
            // first presence_poll_tick aren't dropped due to None overlay.
            let mut initial_peers = std::collections::HashMap::new();
            let mut initial_online = std::collections::HashMap::new();
            if let Some(ref coord) = coordinator {
                if !coord.route_blob.is_empty() {
                    let member = crate::state::OnlineMember {
                        route_blob: coord.route_blob.clone(),
                        last_seen: rekindle_utils::timestamp_secs(),
                    };
                    initial_peers.insert(coord.pseudonym_key.clone(), member.clone());
                    initial_online.insert(coord.pseudonym_key.clone(), member);
                }
            }
            Some(crate::state::GossipOverlay {
                peers: initial_peers,
                online_members: initial_online,
                lamport_counter: 0,
                needs_initial_sync: true,
            })
        },
        slot_keypair: Some(slot_keypair_str),
        manifest_owner_keypair: None,
        channel_log_keys: std::collections::HashMap::new(),
        channel_sequences: std::collections::HashMap::new(),
        pending_syncs: std::collections::HashMap::new(),
        registry_owner_keypair: None,
        slot_seed: Some(slot_seed_hex),
        member_roles: existing_members.iter().map(|m| (m.pseudonym_key.clone(), m.role_ids.clone())).collect(),
        known_members: existing_members.iter().map(|m| m.pseudonym_key.clone()).collect(),
        presence_poll_shutdown_tx: None,
        dht_keepalive_shutdown_tx: None,
    };

    state.communities.write().insert(community_id.to_string(), community);

    // Persist discovered members to SQLite so get_community_members works immediately
    super::presence::sync_members_to_state_and_db(state, community_id, &existing_members);

    // ── CONNECT: start services ──

    // 9. Create coordinator handle (static owner model — joiner is Member, not Coordinator)
    let handle = crate::services::coordinator::create_handle(state, community_id.to_string());
    *handle.role.write() = crate::services::coordinator::CoordinatorRole::Member;
    state.coordinator_services.write().insert(community_id.to_string(), handle);

    // 10. Start presence poll (IMMEDIATE first tick writes our presence + discovers peers)
    //     and DHT keepalive. Presence write is our proof of membership — Veilid validates
    //     the slot keypair against the SMPL schema.
    super::presence::start_presence_poll(state.clone(), community_id.to_string());
    super::keepalive::start_dht_keepalive(state.clone(), community_id.to_string());

    tracing::info!(
        community = %community_id,
        subkey_index = my_subkey_index,
        "self-service join complete — presence poll will write to SMPL registry"
    );

    // 11. Broadcast MemberJoinRequest + MemberJoined via gossip (delayed for route allocation)
    spawn_join_announcements(
        state.clone(),
        community_id.to_string(),
        my_pseudonym_key.unwrap_or_default(),
        my_subkey_index,
    );

    Ok(())
}

/// Construct a default community display name from a (potentially long) ID.
fn default_community_name(community_id: &str) -> String {
    format!("Community {}", &community_id[..8.min(community_id.len())])
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
            .is_some_and(crate::services::coordinator::CoordinatorServiceHandle::is_coordinator)
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

/// Broadcast MemberJoinRequest and MemberJoined after a delay.
///
/// Delayed to allow presence_poll_tick to build the gossip overlay and Veilid
/// to allocate a private route. The route_blob is read AFTER the delay so it
/// reflects the current route, not a potentially-None value from before.
fn spawn_join_announcements(
    state: Arc<AppState>,
    community_id: String,
    pseudonym_key: String,
    subkey_index: u32,
) {
    let display_name = {
        let identity = state.identity.read();
        identity.as_ref().map_or_else(
            || "Unknown".to_string(),
            |id| id.display_name.clone(),
        )
    };
    tokio::spawn(async move {
        // Wait for presence poll to build gossip overlay and route allocation
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;

        // Read route_blob AFTER sleep so Veilid has time to allocate a route.
        let mut our_route = state_helpers::our_route_blob(&state);
        if our_route.is_none() {
            tracing::info!(community = %community_id, "route_blob not yet available, waiting 3s more");
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            our_route = state_helpers::our_route_blob(&state);
        }

        // Write presence WITH route_blob to DHT BEFORE announcing.
        // The initial slot claim wrote route_blob: None — now we overwrite with the real route
        // so other members' presence scans find us with a valid route immediately.
        if our_route.is_some() {
            let (registry_key, slot_keypair_str) = {
                let communities = state.communities.read();
                let cs = communities.get(&community_id);
                (
                    cs.and_then(|c| c.member_registry_key.clone()),
                    cs.and_then(|c| c.slot_keypair.clone()),
                )
            };
            if let (Some(rk), Some(kp_str)) = (registry_key, slot_keypair_str) {
                if let (Some(rc), Ok(kp)) = (
                    state_helpers::routing_context(&state),
                    kp_str.parse::<veilid_core::KeyPair>(),
                ) {
                    let mgr = rekindle_protocol::dht::DHTManager::new(rc);
                    let presence = rekindle_protocol::dht::community::types::MemberPresence {
                        pseudonym_key: pseudonym_key.clone(),
                        status: "online".to_string(),
                        status_message: None,
                        game_info: None,
                        route_blob: our_route.clone(),
                        last_heartbeat: rekindle_utils::timestamp_secs(),
                        is_coordinator: false,
                        coordinator_since: 0,
                        is_archiver: false,
                    };
                    if let Err(e) = rekindle_protocol::dht::community::member_registry::write_member_presence(
                        &mgr, &rk, subkey_index, &presence, kp,
                    ).await {
                        tracing::warn!(error = %e, "failed to write presence with route_blob before announcements");
                    } else {
                        tracing::info!(community = %community_id, "wrote presence with route_blob to DHT");
                    }
                }
            }
        }

        // Broadcast MemberJoinRequest so admin can add us to member index
        let join_envelope = rekindle_protocol::dht::community::envelope::CommunityEnvelope::Control(
            rekindle_protocol::dht::community::envelope::ControlPayload::MemberJoinRequest {
                pseudonym_key: pseudonym_key.clone(),
                display_name: display_name.clone(),
                invite_code: None,
                route_blob: our_route.clone(),
                prekey_bundle: None,
                claimed_subkey_index: Some(subkey_index),
            },
        );
        if let Err(e) = crate::commands::community::send_to_mesh(&state, &community_id, &join_envelope) {
            tracing::debug!(
                community = %community_id,
                error = %e,
                "failed to broadcast MemberJoinRequest — admin will discover via presence scan"
            );
        }

        // Broadcast MemberJoined so existing members see us immediately (not after 60s poll)
        let joined_envelope = rekindle_protocol::dht::community::envelope::CommunityEnvelope::Control(
            rekindle_protocol::dht::community::envelope::ControlPayload::MemberJoined {
                pseudonym_key,
                display_name,
                role_ids: vec![0, 1],
                route_blob: our_route.clone(),
            },
        );
        let _ = crate::commands::community::send_to_mesh(&state, &community_id, &joined_envelope);
    });
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
