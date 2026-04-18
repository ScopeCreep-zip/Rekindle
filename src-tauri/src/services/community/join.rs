use std::sync::Arc;

use rekindle_governance::merge;
use rekindle_secrets::derive;
use rekindle_types::governance::{GovernanceEntry, GovernanceSubkeyPayload};
use rekindle_types::id::PseudonymKey;
use rekindle_types::presence::MemberPresence;
use veilid_core::SetDHTValueOptions;

use crate::state::{AppState, ChannelInfo, ChannelType, CommunityState, GossipOverlay, OnlineMember, RoleDefinition};
use crate::state_helpers;

/// Join a community via self-sovereign SMPL slot claim.
///
/// No coordinator needed. The flow:
/// 1. Open governance record → read all subkeys → CRDT merge
/// 2. Derive pseudonym, check ban list from merged state
/// 3. Scan registry for empty slot → claim via compare-and-swap
/// 4. Bootstrap gossip peers from registry route blobs
/// 5. Start background services
pub async fn join_community(
    state: &Arc<AppState>,
    governance_key_str: &str,
    invite_code: Option<&str>,
) -> Result<(), String> {
    let invite_code =
        invite_code.ok_or("invite code required — community join requires a valid invite link")?;

    let rc = state_helpers::routing_context(state)
        .ok_or("Veilid node not attached — cannot join community")?;

    // ── PHASE 1: Read governance record → CRDT merge ──

    let gov_typed_key = governance_key_str
        .parse::<veilid_core::RecordKey>()
        .map_err(|e| format!("invalid governance key: {e}"))?;

    // Open governance record (read-only — we don't have a writer slot yet)
    let _gov_desc = rc.open_dht_record(gov_typed_key.clone(), None)
        .await
        .map_err(|e| format!("failed to open governance record: {e}"))?;

    // Read all subkeys and collect governance entries
    let mut all_entries: Vec<(PseudonymKey, Vec<GovernanceEntry>)> = Vec::new();
    for subkey in 0..255u32 {
        match rc.get_dht_value(gov_typed_key.clone(), subkey, false).await {
            Ok(Some(value)) if !value.data().is_empty() => {
                // v2.0 wire format: GovernanceSubkeyPayload wraps entries with author pseudonym
                if let Ok(payload) = serde_json::from_slice::<GovernanceSubkeyPayload>(value.data()) {
                    all_entries.push((payload.author_pseudonym, payload.entries));
                }
            }
            _ => {} // Empty or unwritten subkey
        }
    }

    // CRDT merge to get canonical governance state
    let gov_state = merge::merge(&all_entries);

    // Extract community metadata from merged state
    let name = gov_state
        .metadata
        .as_ref()
        .map_or_else(|| default_community_name(governance_key_str), |m| m.name.clone());
    let description = gov_state.metadata.as_ref().and_then(|m| m.description.clone());

    // ── PHASE 2: Derive pseudonym, check bans ──

    let master_secret = {
        let guard = state.identity_secret.lock();
        *guard.as_ref().ok_or("identity secret not available")?
    };
    let pseudonym_signing = derive::derive_community_pseudonym(&master_secret, governance_key_str);
    let my_pseudo_bytes = pseudonym_signing.verifying_key().to_bytes();
    let my_pseudo_hex = hex::encode(my_pseudo_bytes);
    let my_pseudo = PseudonymKey(my_pseudo_bytes);

    // Ban check from CRDT state (client-side, no coordinator)
    if gov_state.bans.contains(&my_pseudo) {
        return Err("You are banned from this community".into());
    }

    // ── PHASE 3: Decrypt invite secrets from governance entries ──

    let code_hash = rekindle_secrets::invite::hash_invite_code(invite_code);

    // Find matching InviteCreated entry in governance state
    let encrypted_b64 = find_invite_in_governance(&all_entries, &code_hash)?;

    // Decrypt with invite code as HKDF key
    let encrypted = {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .decode(&encrypted_b64)
            .map_err(|e| format!("invalid invite secrets encoding: {e}"))?
    };
    let secrets_json = rekindle_secrets::invite::decrypt_invite_secrets(invite_code, &encrypted)
        .map_err(|e| format!("failed to decrypt invite secrets: {e}"))?;
    let secrets: rekindle_types::invite::InviteSecrets = serde_json::from_slice(&secrets_json)
        .map_err(|e| format!("invalid invite secrets: {e}"))?;

    let registry_key = secrets.registry_key;
    let slot_seed_hex = secrets.slot_seed;

    // Cache MEK from invite
    let mek_generation = {
        use base64::Engine;
        let mek_wire = base64::engine::general_purpose::STANDARD
            .decode(&secrets.mek_wire_bytes)
            .map_err(|e| format!("invalid MEK encoding: {e}"))?;
        let mek = rekindle_crypto::group::media_key::MediaEncryptionKey::from_wire_bytes(&mek_wire)
            .ok_or("invalid MEK wire bytes")?;
        let gen = mek.generation();
        state.mek_cache.lock().insert(governance_key_str.to_string(), mek);
        gen
    };

    let slot_seed_bytes: [u8; 32] = hex::decode(&slot_seed_hex)
        .map_err(|e| format!("invalid slot seed hex: {e}"))?
        .try_into()
        .map_err(|_| "slot seed must be 32 bytes")?;

    // ── PHASE 4: Scan registry for empty slot, claim via CAS ──

    let reg_typed_key = registry_key
        .parse::<veilid_core::RecordKey>()
        .map_err(|e| format!("invalid registry key: {e}"))?;

    let _reg_desc = rc.open_dht_record(reg_typed_key.clone(), None)
        .await
        .map_err(|e| format!("failed to open registry: {e}"))?;

    // Scan for empty slot (inspect returns sequence numbers — seq 0 = never written)
    let report = rc
        .inspect_dht_record(
            reg_typed_key.clone(),
            Some(veilid_core::ValueSubkeyRangeSet::full()),
            veilid_core::DHTReportScope::UpdateGet,
        )
        .await
        .map_err(|e| format!("registry inspect failed: {e}"))?;

    let mut my_slot: Option<u32> = None;
    for subkey in 0..255u32 {
        // Network seqs tell us what the DHT network has for each subkey
        if let Some(&seq) = report.network_seqs().get(subkey as usize) {
            if seq == veilid_core::ValueSeqNum::default() {
                my_slot = Some(subkey);
                break;
            }
        }
    }
    let my_slot = my_slot.ok_or("Community is full — all 255 slots occupied")?;

    // Derive slot keypair and claim
    let slot_kp = derive::derive_slot_keypair(&slot_seed_bytes, my_slot)
        .map_err(|e| format!("slot keypair derivation failed: {e}"))?;
    let slot_veilid = super::create::slot_signing_to_veilid(&slot_kp);

    let presence = MemberPresence {
        pseudonym_key: my_pseudo.clone(),
        display_name: Some(state_helpers::identity_display_name(state)),
        status: "online".into(),
        route_blob: vec![], // Filled by presence_poll once route is available
        last_heartbeat: rekindle_utils::timestamp_secs(),
        ..Default::default()
    };
    let presence_bytes = serde_json::to_vec(&presence)
        .map_err(|e| format!("presence serialization failed: {e}"))?;

    // Write to claim the slot
    let write_opts = SetDHTValueOptions {
        writer: Some(slot_veilid.clone()),
        ..Default::default()
    };
    rc.set_dht_value(reg_typed_key.clone(), my_slot, presence_bytes.clone(), Some(write_opts))
        .await
        .map_err(|e| format!("slot claim write failed: {e}"))?;

    // Compare-and-swap: re-read to verify we won the slot
    let verify = rc
        .get_dht_value(reg_typed_key.clone(), my_slot, true)
        .await
        .map_err(|e| format!("slot verification read failed: {e}"))?
        .ok_or("slot read-back returned empty after write")?;
    let written: MemberPresence = serde_json::from_slice(verify.data())
        .map_err(|e| format!("slot read-back deserialization failed: {e}"))?;
    if written.pseudonym_key != my_pseudo {
        return Err("Slot collision — another member claimed this slot. Please retry.".into());
    }

    // ── PHASE 5: Bootstrap gossip peers from registry ──

    let mut initial_peers = std::collections::HashMap::new();
    let mut initial_online = std::collections::HashMap::new();
    let mut known_members = std::collections::HashSet::new();
    known_members.insert(my_pseudo_hex.clone());

    for subkey in 0..255u32 {
        if subkey == my_slot {
            continue;
        }
        if let Ok(Some(val)) = rc.get_dht_value(reg_typed_key.clone(), subkey, false).await {
            if val.data().is_empty() {
                continue;
            }
            if let Ok(p) = serde_json::from_slice::<MemberPresence>(val.data()) {
                let pseudo_hex = hex::encode(p.pseudonym_key.0);
                known_members.insert(pseudo_hex.clone());
                if !p.route_blob.is_empty() {
                    let member = OnlineMember {
                        route_blob: p.route_blob.clone(),
                        last_seen: p.last_heartbeat,
                    };
                    initial_peers.insert(pseudo_hex.clone(), member.clone());
                    initial_online.insert(pseudo_hex, member);
                }
            }
        }
    }

    // ── PHASE 6: Build channels/roles from governance state ──

    let channels: Vec<ChannelInfo> = gov_state
        .channels
        .iter()
        .map(|(ch_id, ch)| ChannelInfo {
            id: hex::encode(ch_id.0),
            name: ch.name.clone(),
            channel_type: ch.channel_type.parse().unwrap_or(ChannelType::Text),
            unread_count: 0,
            category_id: ch.category_id.map(|c| hex::encode(c.0)),
            topic: ch.topic.clone().unwrap_or_default(),
            slowmode_seconds: ch.slowmode_seconds,
            nsfw: ch.nsfw.unwrap_or(false),
            message_record_key: Some(ch.record_key.clone()),
            mek_generation: 0,
        })
        .collect();

    let roles: Vec<RoleDefinition> = gov_state
        .roles
        .iter()
        .map(|(rid, r)| RoleDefinition {
            id: u32::from(rid.0[0]), // Simplified — first byte as legacy ID
            name: r.name.clone(),
            color: r.color,
            permissions: r.permissions,
            position: r.position.cast_signed(),
            hoist: r.hoist,
            mentionable: r.mentionable,
        })
        .collect();

    // Determine our role IDs from governance assignments
    let my_role_ids: Vec<u32> = gov_state
        .role_assignments
        .get(&my_pseudo)
        .map_or_else(|| vec![0], |rids| rids.iter().map(|rid| u32::from(rid.0[0])).collect());

    // ── PHASE 7: Build CommunityState ──

    let channel_log_keys: std::collections::HashMap<String, String> = gov_state
        .channels
        .iter()
        .map(|(ch_id, ch)| (hex::encode(ch_id.0), ch.record_key.clone()))
        .collect();

    let community = CommunityState {
        id: governance_key_str.to_string(),
        name,
        description,
        channels,
        categories: gov_state
            .categories
            .iter()
            .map(|(cat_id, cat)| crate::state::CategoryInfo {
                id: hex::encode(cat_id.0),
                name: cat.name.clone(),
                sort_order: cat.position.try_into().unwrap_or(0),
            })
            .collect(),
        my_role_ids,
        roles,
        my_role: Some("member".to_string()),
        dht_record_key: Some(governance_key_str.to_string()),
        dht_owner_keypair: None,
        my_pseudonym_key: Some(my_pseudo_hex.clone()),
        mek_generation,
        manifest_key: None,
        member_registry_key: Some(registry_key.clone()),
        my_subkey_index: Some(my_slot),
        coordinator_pseudonym: None,
        coordinator_route_blob: None,
        coordinator_epoch: 0,
        governance_key: Some(governance_key_str.to_string()),
        governance_state: Some(gov_state),
        lamport_counter: 0,
        gossip: Some(GossipOverlay {
            peers: initial_peers,
            online_members: initial_online,
            lamport_counter: 0,
            needs_initial_sync: true,
        }),
        slot_keypair: Some(slot_veilid.to_string()),
        manifest_owner_keypair: None,
        channel_log_keys,
        channel_sequences: std::collections::HashMap::new(),
        pending_syncs: std::collections::HashMap::new(),
        peer_sequences: std::collections::HashMap::new(),
        registry_owner_keypair: None,
        slot_seed: Some(slot_seed_hex),
        member_roles: std::collections::HashMap::new(),
        known_members,
        presence_poll_shutdown_tx: None,
        dht_keepalive_shutdown_tx: None,
        open_community_records: crate::state::CommunityRecords::default(),
    };

    state
        .communities
        .write()
        .insert(governance_key_str.to_string(), community);

    // Track opened records
    {
        let mut communities = state.communities.write();
        if let Some(cs) = communities.get_mut(governance_key_str) {
            cs.open_community_records.manifest_key = Some(governance_key_str.to_string());
            cs.open_community_records.registry_key = Some(registry_key);
            let writer = cs.slot_keypair.clone();
            cs.open_community_records.registry_writer = writer;
            let ch_keys: Vec<String> = cs
                .channel_log_keys
                .values()
                .cloned()
                .collect();
            cs.open_community_records.channel_keys = ch_keys;
            cs.open_community_records.records_open = true;
        }
    }

    // Open channel records
    open_channel_records(&rc, state, governance_key_str).await;

    // ── PHASE 8: Start services (no coordinator handle) ──

    // Delayed presence poll — let route allocation settle
    {
        let poll_state = state.clone();
        let poll_cid = governance_key_str.to_string();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            super::presence::start_presence_poll(poll_state, poll_cid);
        });
    }
    super::keepalive::start_dht_keepalive(state.clone(), governance_key_str.to_string());

    // Broadcast join announcements after delay
    spawn_join_announcements(
        state.clone(),
        governance_key_str.to_string(),
        my_pseudo_hex,
        my_slot,
    );

    tracing::info!(
        community = %governance_key_str,
        slot = my_slot,
        "self-sovereign join complete — SMPL slot claimed, gossip peers bootstrapped"
    );

    Ok(())
}

/// Find a matching InviteCreated entry in the governance entries by code hash.
///
/// Scans all governance entries from all member subkeys for InviteCreated
/// that matches the given code_hash. Returns the encrypted_secrets base64 blob.
fn find_invite_in_governance(
    subkeys: &[(PseudonymKey, Vec<GovernanceEntry>)],
    code_hash: &str,
) -> Result<String, String> {
    // Collect all revoked invite IDs first
    let mut revoked_ids: std::collections::HashSet<[u8; 16]> = std::collections::HashSet::new();
    for (_, entries) in subkeys {
        for entry in entries {
            if let GovernanceEntry::InviteRevoked { invite_id, .. } = entry {
                revoked_ids.insert(*invite_id);
            }
        }
    }

    for (_, entries) in subkeys {
        for entry in entries {
            if let GovernanceEntry::InviteCreated {
                invite_id,
                code_hash: ref ch,
                encrypted_secrets,
                expires_at,
                lamport: _,
                ..
            } = entry
            {
                if ch == code_hash {
                    // Check revocation
                    if revoked_ids.contains(invite_id) {
                        return Err("invite has been revoked".into());
                    }
                    // Validate expiry
                    if let Some(exp) = expires_at {
                        if rekindle_utils::timestamp_secs() > *exp {
                            return Err("invite has expired".into());
                        }
                    }
                    return Ok(encrypted_secrets.clone());
                }
            }
        }
    }
    Err("invalid invite code — no matching invite found in governance".into())
}

/// Open channel SMPL records so message writes don't hit "record not open".
async fn open_channel_records(
    rc: &veilid_core::RoutingContext,
    state: &Arc<AppState>,
    community_id: &str,
) {
    let channel_keys: Vec<String> = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .map(|cs| cs.channel_log_keys.values().cloned().collect())
            .unwrap_or_default()
    };

    for key_str in &channel_keys {
        if let Ok(typed_key) = key_str.parse::<veilid_core::RecordKey>() {
            if let Err(e) = rc.open_dht_record(typed_key, None).await {
                tracing::debug!(key = %key_str, error = %e, "failed to open channel record on join");
            }
            // Small delay between opens to avoid connection burst
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }
}

/// Broadcast MemberJoined via gossip after route allocation delay.
fn spawn_join_announcements(
    state: Arc<AppState>,
    community_id: String,
    pseudonym_key: String,
    subkey_index: u32,
) {
    let display_name = state_helpers::identity_display_name(&state);
    tokio::spawn(async move {
        // Wait for presence poll to build gossip overlay
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;

        let our_route = state_helpers::our_route_blob(&state);

        // Broadcast MemberJoined so existing members see us immediately
        let joined_envelope = rekindle_protocol::dht::community::envelope::CommunityEnvelope::Control(
            rekindle_protocol::dht::community::envelope::ControlPayload::MemberJoined {
                pseudonym_key: pseudonym_key.clone(),
                display_name,
                role_ids: vec![0],
                route_blob: our_route,
            },
        );
        let _ = crate::commands::community::send_to_mesh(&state, &community_id, &joined_envelope);

        tracing::info!(
            community = %community_id,
            slot = subkey_index,
            "broadcasted MemberJoined via gossip"
        );
    });
}

/// Construct a default community display name from a (potentially long) ID.
fn default_community_name(governance_key: &str) -> String {
    format!("Community {}", &governance_key[..8.min(governance_key.len())])
}

/// Re-announce our route to the community after restart.
///
/// Broadcasts PresenceUpdate via gossip so all peers learn our fresh route.
/// No coordinator re-fetch — flat governance doesn't need it.
pub async fn rejoin_community(state: &Arc<AppState>, community_id: &str) -> Result<(), String> {
    if crate::state_helpers::is_circuit_open(state, community_id) {
        tracing::debug!(community = %community_id, "skipping rejoin — circuit breaker open");
        return Ok(());
    }

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

/// Try to derive the slot keypair locally from slot_seed + subkey_index.
pub(crate) fn try_derive_slot_keypair(
    state: &Arc<AppState>,
    community_id: &str,
    seed_hex: &str,
    subkey_idx: u32,
) -> Option<String> {
    let seed_bytes = hex::decode(seed_hex).ok()?;
    let seed_array: [u8; 32] = seed_bytes.as_slice().try_into().ok()?;
    match derive::derive_slot_keypair(&seed_array, subkey_idx) {
        Ok(sk) => {
            let kp = super::create::slot_signing_to_veilid(&sk);
            let kp_str = kp.to_string();
            {
                let mut communities = state.communities.write();
                if let Some(c) = communities.get_mut(community_id) {
                    c.slot_keypair = Some(kp_str.clone());
                }
            }
            Some(kp_str)
        }
        Err(e) => {
            tracing::warn!(error = %e, "failed to derive slot keypair from seed");
            None
        }
    }
}
