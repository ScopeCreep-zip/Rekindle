//! Phase 18.h.2 — chiral-split join orchestrator.
//!
//! Protocol primitives (identity, governance snapshot, slot claim,
//! presence collection) live in `rekindle_governance_runtime::join` +
//! `join_stages`. This module is the Tauri-side orchestrator that
//! sequences them, decodes the invite payload (Stronghold + MEK cache),
//! builds the src-tauri `CommunityState`, and spawns the background
//! services.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use rekindle_governance_runtime as gov_rt;
use rekindle_types::id::PseudonymKey;
use tauri::Manager;

use crate::state::{AppState, CommunityState, GossipOverlay, OnlineMember};

use super::bootstrap::{fetch_bootstrap_bundle, BootstrapBundle};
use super::helpers::{open_channel_records, role_id_to_legacy_u32, spawn_join_announcements};
use super::state::{build_channel_log_keys, build_channels, build_roles, join_status_label};

struct InviteContext {
    registry_key: String,
    slot_seed_hex: String,
    bootstrap_bundle: Option<BootstrapBundle>,
    mek_generation: u64,
    inviter_pseudonym: PseudonymKey,
}

pub async fn join_community(
    state: &Arc<AppState>,
    governance_key_str: &str,
    invite_code: Option<&str>,
) -> Result<(), String> {
    let invite_code =
        invite_code.ok_or("invite code required — community join requires a valid invite link")?;

    let app_handle = state
        .app_handle
        .read()
        .clone()
        .ok_or_else(|| "app handle unavailable".to_string())?;
    let pool = app_handle
        .try_state::<crate::db::DbPool>()
        .ok_or_else(|| "DbPool state missing".to_string())?
        .inner()
        .clone();
    let adapter = crate::services::governance_adapter::GovernanceAdapter::new(
        Arc::clone(state),
        app_handle,
        pool,
    );

    // 1. Multi-segment governance snapshot via crate primitive (DHT scan +
    //    W26 signature verify + multi-segment re-merge).
    let snapshot = gov_rt::load_governance_snapshot(&adapter, governance_key_str)
        .await
        .map_err(|e| e.to_string())?;

    // 2. Joiner identity derived from master secret + community key.
    let identity_secret = state
        .identity_secret
        .lock()
        .as_ref()
        .copied()
        .ok_or("identity secret not available")?;
    let identity = gov_rt::derive_join_identity(&identity_secret, governance_key_str);

    if snapshot.gov_state.bans.contains(&identity.pseudo) {
        return Err("You are banned from this community".into());
    }

    // 3. Decode the invite payload + optionally fetch bootstrap bundle.
    let invite = decode_invite_context(
        state,
        governance_key_str,
        invite_code,
        &snapshot.all_entries,
        &identity.pseudo_hex,
    )
    .await?;

    // 4. Claim a registry slot (with auto-expand fallback).
    let display_name = Some(crate::state_helpers::identity_display_name(state));
    let claimed = gov_rt::claim_registry_slot(
        &adapter,
        governance_key_str,
        &invite.registry_key,
        &invite.slot_seed_hex,
        &invite.inviter_pseudonym,
        &identity.pseudo,
        &identity.pseudonym_signing,
        &snapshot.gov_state,
        join_status_label(state),
        display_name,
    )
    .await
    .map_err(|e| e.to_string())?;

    // 5. Initial presence state from a DHT registry scan.
    let initial_presence = gov_rt::collect_initial_presence_state(
        &adapter,
        &claimed.registry_key,
        claimed.local_subkey,
        &identity.pseudo_hex,
    )
    .await;

    // 6. Build the src-tauri CommunityState from all the gathered pieces.
    let channels = build_channels(&snapshot.gov_state);
    let roles = build_roles(&snapshot.gov_state);
    let my_role_ids = snapshot
        .gov_state
        .role_assignments
        .get(&identity.pseudo)
        .map_or_else(
            || vec![0],
            |rids| rids.iter().map(role_id_to_legacy_u32).collect(),
        );
    let channel_log_keys = build_channel_log_keys(&snapshot.gov_state);

    let initial_peers: HashMap<String, OnlineMember> = initial_presence
        .peers
        .iter()
        .map(|(pseudo_hex, m)| {
            (
                pseudo_hex.clone(),
                OnlineMember {
                    route_blob: m.route_blob.clone(),
                    status: m.status.clone(),
                    last_seen: m.last_seen,
                },
            )
        })
        .collect();
    let initial_online: HashMap<String, OnlineMember> = initial_presence
        .online
        .iter()
        .map(|(pseudo_hex, m)| {
            (
                pseudo_hex.clone(),
                OnlineMember {
                    route_blob: m.route_blob.clone(),
                    status: m.status.clone(),
                    last_seen: m.last_seen,
                },
            )
        })
        .collect();
    let known_members: HashSet<String> = initial_presence.known_members.iter().cloned().collect();

    let community = CommunityState {
        id: governance_key_str.to_string(),
        name: snapshot.name,
        description: snapshot.description,
        icon_hash: snapshot
            .gov_state
            .metadata
            .as_ref()
            .and_then(|m| m.icon_hash.clone()),
        banner_hash: snapshot
            .gov_state
            .metadata
            .as_ref()
            .and_then(|m| m.banner_hash.clone()),
        channels,
        categories: snapshot
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
        dht_owner_keypair: None,
        my_pseudonym_key: Some(identity.pseudo_hex.clone()),
        mek_generation: invite.mek_generation,
        member_registry_key: Some(invite.registry_key.clone()),
        my_subkey_index: Some(claimed.local_subkey),
        my_segment_index: Some(claimed.segment_index),
        governance_key: Some(governance_key_str.to_string()),
        governance_state: Some(snapshot.gov_state),
        lamport_counter: 0,
        gossip: Some(GossipOverlay {
            peers: initial_peers,
            online_members: initial_online,
            lamport_counter: 0,
            needs_initial_sync: true,
            pending_mesh_broadcasts: std::collections::VecDeque::with_capacity(16),
        }),
        slot_keypair: Some(claimed.slot_keypair_str),
        channel_log_keys,
        channel_sequences: HashMap::new(),
        pending_syncs: HashMap::new(),
        watched_records: HashSet::new(),
        record_sequences: HashMap::new(),
        peer_sequences: HashMap::new(),
        channel_last_send_at: HashMap::new(),
        peer_reliability: HashMap::new(),
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
        my_bio: None,
        my_pronouns: None,
        my_theme_color: None,
        my_badges: Vec::new(),
        my_avatar_ref: None,
        my_banner_ref: None,
        member_profiles: HashMap::new(),
        recent_member_joins: std::collections::VecDeque::new(),
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

    let rc = crate::state_helpers::safe_routing_context(state).ok_or("not attached")?;
    open_channel_records(&rc, state, governance_key_str).await;
    if let Err(e) = super::super::files::ensure_cache_open(state, governance_key_str) {
        tracing::warn!(community = %governance_key_str, error = %e, "Lost Cargo cache unavailable on join");
    }
    super::super::files::sync_pinned_from_governance(state, governance_key_str);
    let _ = super::super::watch::watch_community_records(state, governance_key_str).await;
    super::super::inspect::start_inspect_loop(state.clone(), governance_key_str.to_string());

    {
        let poll_state = state.clone();
        let poll_cid = governance_key_str.to_string();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            super::super::presence::start_presence_poll(&poll_state, poll_cid);
        });
    }
    super::super::keepalive::start_dht_keepalive(state.clone(), governance_key_str.to_string());
    super::history::schedule_history_catchup(state.clone(), governance_key_str.to_string());

    if let Some(ref bundle) = invite.bootstrap_bundle {
        super::bootstrap::persist_bootstrap_recent_messages(state, governance_key_str, bundle).await;
    }

    spawn_join_announcements(
        state.clone(),
        governance_key_str.to_string(),
        identity.pseudo_hex,
        claimed.local_subkey,
    );

    tracing::info!(
        community = %governance_key_str,
        slot = claimed.local_subkey,
        "self-sovereign join complete — SMPL slot claimed, gossip peers bootstrapped"
    );

    Ok(())
}

async fn decode_invite_context(
    state: &Arc<AppState>,
    governance_key_str: &str,
    invite_code: &str,
    all_entries: &[(PseudonymKey, Vec<rekindle_types::governance::GovernanceEntry>)],
    pseudo_hex: &str,
) -> Result<InviteContext, String> {
    let code_hash = rekindle_secrets::invite::hash_invite_code(invite_code);
    let (encrypted_b64, inviter_pseudonym) =
        gov_rt::find_invite_in_entries(all_entries, &code_hash).map_err(|e| e.to_string())?;
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
        inviter_pseudonym,
    })
}
