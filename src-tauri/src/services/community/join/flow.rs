use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use rekindle_governance::merge;
use rekindle_secrets::derive;
use rekindle_types::governance::{GovernanceEntry, GovernanceSubkeyPayload};
use rekindle_types::id::PseudonymKey;
use rekindle_types::presence::MemberPresence;
use veilid_core::{RecordKey, SetDHTValueOptions};

use crate::state::{AppState, CommunityState, GossipOverlay, OnlineMember};
use crate::state_helpers;

use super::bootstrap::{fetch_bootstrap_bundle, BootstrapBundle};
use super::helpers::{
    default_community_name, find_invite_in_governance, open_channel_records, role_id_to_legacy_u32,
    spawn_join_announcements,
};
use super::state::{build_channel_log_keys, build_channels, build_roles, join_status_label};

struct GovernanceSnapshot {
    all_entries: Vec<(PseudonymKey, Vec<GovernanceEntry>)>,
    gov_state: rekindle_governance::state::GovernanceState,
    name: String,
    description: Option<String>,
}

struct JoinIdentity {
    pseudo_hex: String,
    pseudo: PseudonymKey,
}

struct InviteContext {
    registry_key: String,
    slot_seed_hex: String,
    bootstrap_bundle: Option<BootstrapBundle>,
    mek_generation: u64,
}

pub async fn join_community(
    state: &Arc<AppState>,
    governance_key_str: &str,
    invite_code: Option<&str>,
) -> Result<(), String> {
    let invite_code =
        invite_code.ok_or("invite code required — community join requires a valid invite link")?;
    let rc = state_helpers::safe_routing_context(state)
        .ok_or("Veilid node not attached — cannot join community")?;

    let governance = load_governance_snapshot(&rc, governance_key_str).await?;
    let identity = derive_join_identity(state, governance_key_str)?;
    if governance.gov_state.bans.contains(&identity.pseudo) {
        return Err("You are banned from this community".into());
    }

    let invite = decode_invite_context(
        state,
        governance_key_str,
        invite_code,
        &governance.all_entries,
        &identity.pseudo_hex,
    )
    .await?;
    let (registry_typed_key, my_slot, slot_veilid) = claim_registry_slot(
        state,
        &rc,
        &invite.registry_key,
        &invite.slot_seed_hex,
        &identity.pseudo,
    )
    .await?;
    let (initial_peers, initial_online, known_members) = collect_initial_presence_state(
        &rc,
        &registry_typed_key,
        my_slot,
        invite.bootstrap_bundle.as_ref(),
        &identity.pseudo_hex,
    )
    .await;

    let channels = build_channels(&governance.gov_state);
    let roles = build_roles(&governance.gov_state);
    let my_role_ids = governance
        .gov_state
        .role_assignments
        .get(&identity.pseudo)
        .map_or_else(
            || vec![0],
            |rids| rids.iter().map(role_id_to_legacy_u32).collect(),
        );
    let channel_log_keys = build_channel_log_keys(&governance.gov_state);

    let community = CommunityState {
        id: governance_key_str.to_string(),
        name: governance.name,
        description: governance.description,
        channels,
        categories: governance
            .gov_state
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
        dht_owner_keypair: None,
        my_pseudonym_key: Some(identity.pseudo_hex.clone()),
        mek_generation: invite.mek_generation,
        member_registry_key: Some(invite.registry_key.clone()),
        my_subkey_index: Some(my_slot),
        governance_key: Some(governance_key_str.to_string()),
        governance_state: Some(governance.gov_state),
        lamport_counter: 0,
        gossip: Some(GossipOverlay {
            peers: initial_peers,
            online_members: initial_online,
            lamport_counter: 0,
            needs_initial_sync: true,
        }),
        slot_keypair: Some(slot_veilid.to_string()),
        channel_log_keys,
        channel_sequences: HashMap::new(),
        pending_syncs: HashMap::new(),
        watched_records: HashSet::new(),
        record_sequences: HashMap::new(),
        peer_sequences: HashMap::new(),
        registry_owner_keypair: None,
        slot_seed: Some(invite.slot_seed_hex.clone()),
        member_roles: HashMap::new(),
        known_members,
        presence_poll_shutdown_tx: None,
        dht_keepalive_shutdown_tx: None,
        open_community_records: crate::state::CommunityRecords::default(),
        my_event_rsvps: HashMap::new(),
        event_rsvps_by_event: HashMap::new(),
        onboarding_complete: false,
    };

    state
        .communities
        .write()
        .insert(governance_key_str.to_string(), community);

    {
        let mut communities = state.communities.write();
        if let Some(cs) = communities.get_mut(governance_key_str) {
            cs.open_community_records.governance_key = Some(governance_key_str.to_string());
            cs.open_community_records.registry_key = Some(invite.registry_key);
            cs.open_community_records.registry_writer = cs.slot_keypair.clone();
            cs.open_community_records.channel_keys =
                cs.channel_log_keys.values().cloned().collect();
            cs.open_community_records.records_open = true;
        }
    }

    open_channel_records(&rc, state, governance_key_str).await;
    let _ = super::super::watch::watch_community_records(state, governance_key_str).await;
    super::super::inspect::start_inspect_loop(state.clone(), governance_key_str.to_string());

    {
        let poll_state = state.clone();
        let poll_cid = governance_key_str.to_string();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            super::super::presence::start_presence_poll(poll_state, poll_cid);
        });
    }
    super::super::keepalive::start_dht_keepalive(state.clone(), governance_key_str.to_string());
    super::history::schedule_history_catchup(state.clone(), governance_key_str.to_string());

    spawn_join_announcements(
        state.clone(),
        governance_key_str.to_string(),
        identity.pseudo_hex,
        my_slot,
    );

    tracing::info!(
        community = %governance_key_str,
        slot = my_slot,
        "self-sovereign join complete — SMPL slot claimed, gossip peers bootstrapped"
    );

    Ok(())
}

async fn load_governance_snapshot(
    rc: &veilid_core::RoutingContext,
    governance_key_str: &str,
) -> Result<GovernanceSnapshot, String> {
    let gov_typed_key = governance_key_str
        .parse::<RecordKey>()
        .map_err(|e| format!("invalid governance key: {e}"))?;
    let _gov_desc = rc
        .open_dht_record(gov_typed_key.clone(), None)
        .await
        .map_err(|e| format!("failed to open governance record: {e}"))?;

    let mut all_entries = Vec::new();
    for subkey in 0..255u32 {
        match rc.get_dht_value(gov_typed_key.clone(), subkey, false).await {
            Ok(Some(value)) if !value.data().is_empty() => {
                if let Ok(payload) = serde_json::from_slice::<GovernanceSubkeyPayload>(value.data())
                {
                    all_entries.push((payload.author_pseudonym, payload.entries));
                }
            }
            _ => {}
        }
    }

    let gov_state = merge::merge(&all_entries);
    let name = gov_state.metadata.as_ref().map_or_else(
        || default_community_name(governance_key_str),
        |metadata| metadata.name.clone(),
    );
    let description = gov_state
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.description.clone());

    Ok(GovernanceSnapshot {
        all_entries,
        gov_state,
        name,
        description,
    })
}

fn derive_join_identity(
    state: &Arc<AppState>,
    governance_key_str: &str,
) -> Result<JoinIdentity, String> {
    let master_secret = {
        let guard = state.identity_secret.lock();
        *guard.as_ref().ok_or("identity secret not available")?
    };
    let pseudonym_signing = derive::derive_community_pseudonym(&master_secret, governance_key_str);
    let pseudo_bytes = pseudonym_signing.verifying_key().to_bytes();
    Ok(JoinIdentity {
        pseudo_hex: hex::encode(pseudo_bytes),
        pseudo: PseudonymKey(pseudo_bytes),
    })
}

async fn decode_invite_context(
    state: &Arc<AppState>,
    governance_key_str: &str,
    invite_code: &str,
    all_entries: &[(PseudonymKey, Vec<GovernanceEntry>)],
    pseudo_hex: &str,
) -> Result<InviteContext, String> {
    let code_hash = rekindle_secrets::invite::hash_invite_code(invite_code);
    let encrypted_b64 = find_invite_in_governance(all_entries, &code_hash)?;
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

    let bootstrap_bundle = if secrets.inviter_route_blob.is_empty() {
        None
    } else {
        match fetch_bootstrap_bundle(
            state,
            governance_key_str,
            &secrets.inviter_route_blob,
            pseudo_hex,
        )
        .await
        {
            Ok(bundle) => Some(bundle),
            Err(error) => {
                tracing::debug!(
                    community = %governance_key_str,
                    error = %error,
                    "bootstrap bundle unavailable; proceeding with DHT verification path"
                );
                None
            }
        }
    };

    let mek_generation = {
        use base64::Engine;
        let mek_wire = base64::engine::general_purpose::STANDARD
            .decode(&secrets.mek_wire_bytes)
            .map_err(|e| format!("invalid MEK encoding: {e}"))?;
        let mek = rekindle_crypto::group::media_key::MediaEncryptionKey::from_wire_bytes(&mek_wire)
            .ok_or("invalid MEK wire bytes")?;
        let generation = mek.generation();
        state
            .mek_cache
            .lock()
            .insert(governance_key_str.to_string(), mek);
        generation
    };

    Ok(InviteContext {
        registry_key: secrets.registry_key,
        slot_seed_hex: secrets.slot_seed,
        bootstrap_bundle,
        mek_generation,
    })
}

async fn claim_registry_slot(
    state: &Arc<AppState>,
    rc: &veilid_core::RoutingContext,
    registry_key: &str,
    slot_seed_hex: &str,
    my_pseudo: &PseudonymKey,
) -> Result<(RecordKey, u32, veilid_core::KeyPair), String> {
    let slot_seed_bytes: [u8; 32] = hex::decode(slot_seed_hex)
        .map_err(|e| format!("invalid slot seed hex: {e}"))?
        .try_into()
        .map_err(|_| "slot seed must be 32 bytes")?;
    let registry_typed_key = registry_key
        .parse::<RecordKey>()
        .map_err(|e| format!("invalid registry key: {e}"))?;

    let _reg_desc = rc
        .open_dht_record(registry_typed_key.clone(), None)
        .await
        .map_err(|e| format!("failed to open registry: {e}"))?;
    let report = rc
        .inspect_dht_record(
            registry_typed_key.clone(),
            Some(veilid_core::ValueSubkeyRangeSet::full()),
            veilid_core::DHTReportScope::UpdateGet,
        )
        .await
        .map_err(|e| format!("registry inspect failed: {e}"))?;

    let my_slot = (0..255u32)
        .find(|subkey| {
            report
                .network_seqs()
                .get(*subkey as usize)
                .is_some_and(|seq| *seq == veilid_core::ValueSeqNum::default())
        })
        .ok_or("Community is full — all 255 slots occupied")?;

    let slot_kp = derive::derive_slot_keypair(&slot_seed_bytes, my_slot)
        .map_err(|e| format!("slot keypair derivation failed: {e}"))?;
    let slot_veilid = super::super::create::slot_signing_to_veilid(&slot_kp);
    let presence = MemberPresence {
        pseudonym_key: my_pseudo.clone(),
        display_name: Some(state_helpers::identity_display_name(state)),
        status: join_status_label(state).into(),
        route_blob: vec![],
        last_heartbeat: rekindle_utils::timestamp_secs(),
        ..Default::default()
    };
    let presence_bytes =
        serde_json::to_vec(&presence).map_err(|e| format!("presence serialization failed: {e}"))?;
    rc.set_dht_value(
        registry_typed_key.clone(),
        my_slot,
        presence_bytes,
        Some(SetDHTValueOptions {
            writer: Some(slot_veilid.clone()),
            ..Default::default()
        }),
    )
    .await
    .map_err(|e| format!("slot claim write failed: {e}"))?;

    let verify = rc
        .get_dht_value(registry_typed_key.clone(), my_slot, true)
        .await
        .map_err(|e| format!("slot verification read failed: {e}"))?
        .ok_or("slot read-back returned empty after write")?;
    let written: MemberPresence = serde_json::from_slice(verify.data())
        .map_err(|e| format!("slot read-back deserialization failed: {e}"))?;
    if written.pseudonym_key != *my_pseudo {
        return Err("Slot collision — another member claimed this slot. Please retry.".into());
    }

    Ok((registry_typed_key, my_slot, slot_veilid))
}

async fn collect_initial_presence_state(
    rc: &veilid_core::RoutingContext,
    registry_typed_key: &RecordKey,
    my_slot: u32,
    bootstrap_bundle: Option<&BootstrapBundle>,
    my_pseudo_hex: &str,
) -> (
    HashMap<String, OnlineMember>,
    HashMap<String, OnlineMember>,
    HashSet<String>,
) {
    let mut initial_peers = HashMap::new();
    let mut initial_online = HashMap::new();
    let mut known_members = HashSet::new();
    known_members.insert(my_pseudo_hex.to_string());

    if let Some(bundle) = bootstrap_bundle {
        tracing::info!(
            governance_entries = bundle.governance_entry_count,
            member_list = bundle.member_list.len(),
            channel_meks = bundle.channel_mek_count,
            recent_messages = bundle.recent_message_count,
            has_wrapped_owner_keypair = bundle.has_wrapped_owner_keypair,
            "bootstrap bundle received; warming peer state before DHT verification"
        );
        for member in &bundle.member_list {
            merge_presence_entry(
                &mut initial_peers,
                &mut initial_online,
                &mut known_members,
                &member.pseudonym_key,
                &member.status,
                &member.route_blob,
                member.last_seen,
            );
        }
    }

    for subkey in 0..255u32 {
        if subkey == my_slot {
            continue;
        }
        if let Ok(Some(val)) = rc
            .get_dht_value(registry_typed_key.clone(), subkey, false)
            .await
        {
            if val.data().is_empty() {
                continue;
            }
            if let Ok(presence) = serde_json::from_slice::<MemberPresence>(val.data()) {
                let pseudo_hex = hex::encode(presence.pseudonym_key.0);
                merge_presence_entry(
                    &mut initial_peers,
                    &mut initial_online,
                    &mut known_members,
                    &pseudo_hex,
                    &presence.status,
                    &presence.route_blob,
                    presence.last_heartbeat,
                );
            }
        }
    }

    (initial_peers, initial_online, known_members)
}

fn merge_presence_entry(
    initial_peers: &mut HashMap<String, OnlineMember>,
    initial_online: &mut HashMap<String, OnlineMember>,
    known_members: &mut HashSet<String>,
    pseudonym_key: &str,
    status: &str,
    route_blob: &[u8],
    last_seen: u64,
) {
    known_members.insert(pseudonym_key.to_string());
    if status == "offline" || route_blob.is_empty() {
        return;
    }
    let member = OnlineMember {
        route_blob: route_blob.to_vec(),
        status: status.to_string(),
        last_seen,
    };
    initial_peers.insert(pseudonym_key.to_string(), member.clone());
    initial_online.insert(pseudonym_key.to_string(), member);
}
