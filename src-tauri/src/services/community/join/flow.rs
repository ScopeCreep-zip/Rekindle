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
use rekindle_governance_runtime::GovernanceRuntimeDeps;
use rekindle_types::id::PseudonymKey;
use tauri::Manager;

use crate::state::{AppState, CommunityState, GossipOverlay, OnlineMember};

use super::bootstrap::{fetch_bootstrap_bundle, BootstrapBundle};
use super::helpers::{role_id_to_legacy_u32, spawn_join_announcements};
use super::state::{build_channel_log_keys, build_channels, build_roles, join_status_label};

struct InviteContext {
    registry_key: String,
    slot_seed_hex: String,
    bootstrap_bundle: Option<BootstrapBundle>,
    mek_generation: u64,
    inviter_pseudonym: Option<PseudonymKey>,
}

pub async fn join_community(
    state: &Arc<AppState>,
    governance_key_str: &str,
    invite_code: Option<&str>,
    secrets_record_key: Option<&str>,
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
    //    W26 signature verify + multi-segment re-merge), under its own gate.
    let snapshot = gov_rt::gate(
        &adapter,
        governance_key_str,
        gov_rt::JoinPhase::GovernanceSnapshot,
        async {
            gov_rt::load_governance_snapshot(&adapter, governance_key_str)
                .await
                .map_err(|e| e.to_string())
        },
    )
    .await?;

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
    let invite = gov_rt::gate(
        &adapter,
        governance_key_str,
        gov_rt::JoinPhase::DecodeInvite,
        decode_invite_context(
            state,
            &adapter,
            governance_key_str,
            invite_code,
            secrets_record_key,
            &snapshot.all_entries,
            &identity.pseudo_hex,
        ),
    )
    .await?;

    // 4. Claim a registry slot (with auto-expand fallback), under its gate.
    let display_name = Some(crate::state_helpers::identity_display_name(state));
    let claimed = gov_rt::gate(
        &adapter,
        governance_key_str,
        gov_rt::JoinPhase::ClaimSlot,
        async {
            gov_rt::claim_registry_slot(
                &adapter,
                &invite.slot_seed_hex,
                gov_rt::SlotClaimCtx {
                    community_id: governance_key_str,
                    invite_registry_key: &invite.registry_key,
                    inviter_pseudonym: invite.inviter_pseudonym.as_ref(),
                    my_pseudo: &identity.pseudo,
                    pseudonym_signing: &identity.pseudonym_signing,
                    gov_state: &snapshot.gov_state,
                    join_status_label: join_status_label(state),
                    display_name,
                },
            )
            .await
            .map_err(|e| e.to_string())
        },
    )
    .await?;

    // 5. Initial presence state from a DHT registry scan (infallible, but
    //    gated so a hung registry scan times out instead of stalling).
    let initial_presence = gov_rt::gate(
        &adapter,
        governance_key_str,
        gov_rt::JoinPhase::CollectPresence,
        async {
            Ok(gov_rt::collect_initial_presence_state(
                &adapter,
                &claimed.registry_key,
                claimed.local_subkey,
                &identity.pseudo_hex,
                &claimed.occupied_subkeys,
            )
            .await)
        },
    )
    .await?;

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

    // Durable roster (architecture §13.4): turn every member discovered in the
    // cold-join registry scan — plus our own freshly-claimed row — into
    // `community_members` rows. `get_community_members` reads that table, so
    // without this a fresh joiner sees an empty member list even though the
    // gossip overlay (below) is seeded; the overlay is only the *ephemeral*
    // online layer that decorates table rows. Built here, before
    // `snapshot.gov_state` is moved into `CommunityState`, so role assignments
    // are still readable; persisted after the community is inserted (the
    // persister resolves the community by id).
    let discovered_members = super::helpers::build_discovered_roster(
        &claimed,
        &initial_presence,
        my_role_ids.clone(),
        &snapshot.gov_state,
    );

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
                    ..Default::default()
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
                    ..Default::default()
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
        // Seed the GovernanceOverflow records discovered during the snapshot so
        // OpenRecords (§10) opens+tracks them and keepalive (§14.1) warms them —
        // the joiner can't register via community_id before this insert.
        open_community_records: crate::state::CommunityRecords {
            governance_overflow_keys: snapshot.overflow_keys,
            ..crate::state::CommunityRecords::default()
        },
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
        my_session_location: None,
        presence_policy: rekindle_types::presence::PresenceSharingPolicy::default(),
    };

    state
        .communities
        .write()
        .insert(governance_key_str.to_string(), community);

    // Persist the durable roster now that the community is in the state map
    // (the persister resolves the community by id, fires `MemberDiscovered`
    // per newly-seen pseudonym, and batches the SQLite upsert). The joiner now
    // sees the full membership — themselves + every scanned peer — immediately,
    // independent of route allocation or the +10s steady poll.
    adapter.persist_discovered_registry_members(governance_key_str, discovered_members);

    // 7-8. Post-commit record reconciliation (architecture §6.2 Steps 11/13).
    //    The join COMMITTED at the slot claim above: every input the user needs
    //    to *interact* — full governance state (channels/roles/categories), the
    //    channel MEK (cached in DecodeInvite), the claimed registry slot +
    //    `slot_keypair`, the channel-log keys, and the gossip overlay — is
    //    already installed in the `CommunityState` inserted above. Opening the
    //    channel/overflow records (Step 11), establishing DHT watches (Step 13),
    //    warming the file cache, and history catch-up (Step 15) are best-effort
    //    RECONCILIATION, not prerequisites: the governance + registry records
    //    were already opened during the snapshot read and the slot claim, the
    //    send path opens channel records on demand and queues+retries failed
    //    writes, and the inspect / keepalive / login-rehydration loops re-open
    //    and re-watch anything that lags. So these steps run under their dial-in
    //    gates purely to stream progress — their failure is logged and NEVER
    //    rolls back the committed membership (a slow or unreachable record must
    //    not un-join a fully-equipped member). This uses the same swallowing
    //    discipline as the login hydration path's `open_and_track_one_community`.
    if let Some(setup) = adapter
        .list_communities_for_dht_open()
        .into_iter()
        .find(|s| s.id == governance_key_str)
    {
        let _ = gov_rt::gate(
            &adapter,
            governance_key_str,
            gov_rt::JoinPhase::OpenRecords,
            async {
                gov_rt::open_and_track_one_community(&adapter, &setup).await;
                Ok(())
            },
        )
        .await;

        if let Err(e) = super::super::files::ensure_cache_open(state, governance_key_str) {
            tracing::warn!(community = %governance_key_str, error = %e, "Lost Cargo cache unavailable on join");
        }
        super::super::files::sync_pinned_from_governance(state, governance_key_str);

        if let Err(e) = gov_rt::gate(
            &adapter,
            governance_key_str,
            gov_rt::JoinPhase::WatchRecords,
            super::super::watch::watch_community_records(state, governance_key_str),
        )
        .await
        {
            tracing::warn!(
                community = %governance_key_str,
                error = %e,
                "watch setup deferred to inspect-loop recovery"
            );
        }
    } else {
        tracing::warn!(
            community = %governance_key_str,
            "dht-open setup missing after join; records will open on next login rehydration"
        );
    }

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
        tracing::info!(
            community = %governance_key_str,
            members = bundle.member_list.len(),
            governance_entries = bundle.governance_entry_count,
            channel_meks = bundle.channel_mek_count,
            wrapped_owner_keypair = bundle.has_wrapped_owner_keypair,
            "applying §14.4 bootstrap bundle"
        );
        super::bootstrap::persist_bootstrap_members(state, governance_key_str, bundle).await;
        super::bootstrap::persist_bootstrap_recent_messages(state, governance_key_str, bundle)
            .await;
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
    adapter: &crate::services::governance_adapter::GovernanceAdapter,
    governance_key_str: &str,
    invite_code: &str,
    link_secrets_record_key: Option<&str>,
    all_entries: &[(
        PseudonymKey,
        Vec<rekindle_types::governance::GovernanceEntry>,
    )],
    pseudo_hex: &str,
) -> Result<InviteContext, String> {
    let code_hash = rekindle_secrets::invite::hash_invite_code(invite_code);
    // Chiral §12: secrets are decrypted directly from the invite. Governance is
    // read only to ENFORCE revocation/expiry when visible and to recover the
    // inviter pseudonym for the best-effort quota sanity check — never as the
    // sole source of the secrets pointer (which now rides in the deep link).
    let status = gov_rt::inspect_invite_in_entries(all_entries, &code_hash);
    if matches!(status, gov_rt::InviteGovStatus::Revoked) {
        return Err("invite has been revoked".into());
    }
    if matches!(status, gov_rt::InviteGovStatus::Expired) {
        return Err("invite has expired".into());
    }
    let secrets_record_key = link_secrets_record_key
        .map(str::to_string)
        .or_else(|| match &status {
            gov_rt::InviteGovStatus::Active {
                secrets_record_key, ..
            } => Some(secrets_record_key.clone()),
            _ => None,
        })
        .ok_or("invite has no secrets pointer (no link pointer and no governance entry)")?;
    let inviter_pseudonym = match status {
        gov_rt::InviteGovStatus::Active { inviter, .. } => Some(inviter),
        _ => None,
    };
    // Governance carries only a pointer; fetch the encrypted blob from the
    // invite-secrets DFLT record before decrypting.
    let encrypted_b64 = gov_rt::fetch_invite_secrets(adapter, &secrets_record_key)
        .await
        .map_err(|e| e.to_string())?;
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
        // Centralized resolver — the invite-delivered MEK carries provenance
        // (from_wire_bytes); routing it through the resolver keeps a join that
        // races a live rotation convergent instead of clobbering.
        crate::state_helpers::install_community_mek(state, governance_key_str, mek);
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
