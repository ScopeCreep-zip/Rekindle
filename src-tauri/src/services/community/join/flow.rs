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
    pseudonym_signing: ed25519_dalek::SigningKey,
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
    let (registry_typed_key, claimed_segment, my_slot, slot_veilid) = claim_registry_slot(
        state,
        &rc,
        &invite.registry_key,
        &invite.slot_seed_hex,
        &identity.pseudo,
        &identity.pseudonym_signing,
        &governance.gov_state.segments,
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
        icon_hash: governance
            .gov_state
            .metadata
            .as_ref()
            .and_then(|m| m.icon_hash.clone()),
        banner_hash: governance
            .gov_state
            .metadata
            .as_ref()
            .and_then(|m| m.banner_hash.clone()),
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
        dht_owner_keypair: None,
        my_pseudonym_key: Some(identity.pseudo_hex.clone()),
        mek_generation: invite.mek_generation,
        member_registry_key: Some(invite.registry_key.clone()),
        my_subkey_index: Some(my_slot),
        my_segment_index: Some(claimed_segment),
        governance_key: Some(governance_key_str.to_string()),
        governance_state: Some(governance.gov_state),
        lamport_counter: 0,
        gossip: Some(GossipOverlay {
            peers: initial_peers,
            online_members: initial_online,
            lamport_counter: 0,
            needs_initial_sync: true,
            pending_mesh_broadcasts: std::collections::VecDeque::with_capacity(16),
        }),
        slot_keypair: Some(slot_veilid.to_string()),
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
            super::super::presence::start_presence_poll(poll_state, poll_cid);
        });
    }
    super::super::keepalive::start_dht_keepalive(state.clone(), governance_key_str.to_string());
    super::history::schedule_history_catchup(state.clone(), governance_key_str.to_string());
    // Architecture §14.4: persist the bundle's recent_messages once
    // MEKs have been distributed (the JoinAccepted handler delivers
    // them into channel_mek_cache earlier in this same flow), so the
    // joiner sees scrollback immediately.
    if let Some(ref bundle) = invite.bootstrap_bundle {
        super::bootstrap::persist_bootstrap_recent_messages(
            state,
            governance_key_str,
            bundle,
        )
        .await;
    }

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
    // First-pass: fetch + merge from the primary (segment 0) governance.
    let mut all_entries = fetch_governance_record_entries(rc, governance_key_str).await?;
    let gov_state_v1 = merge::merge(&all_entries);

    // Second-pass: any segments announced in v1's merged state get
    // fetched + their entries appended. CRDT idempotence + commutativity
    // (Almeida 2016 §3) guarantees the re-merge is canonical regardless
    // of fetch order.
    for segment in &gov_state_v1.segments {
        if segment.segment_index == 0 {
            continue;
        }
        match fetch_governance_record_entries(rc, &segment.governance_key).await {
            Ok(mut extra) => all_entries.append(&mut extra),
            Err(e) => tracing::warn!(
                segment = segment.segment_index,
                governance_key = %segment.governance_key,
                error = %e,
                "load_governance_snapshot: skipping unreachable segment governance record"
            ),
        }
    }
    let gov_state = if gov_state_v1.segments.iter().any(|s| s.segment_index > 0) {
        merge::merge(&all_entries)
    } else {
        gov_state_v1
    };

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

async fn fetch_governance_record_entries(
    rc: &veilid_core::RoutingContext,
    governance_key_str: &str,
) -> Result<Vec<(PseudonymKey, Vec<rekindle_types::governance::GovernanceEntry>)>, String> {
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
                    // Architecture §26 W26 — verify the author signature
                    // before trusting the payload. The slot keypair on the
                    // SMPL write is community-shared, so any member could
                    // otherwise impersonate any other.
                    let Ok(sig_arr): Result<[u8; 64], _> =
                        payload.signature.as_slice().try_into()
                    else {
                        continue;
                    };
                    if rekindle_secrets::derive::verify_pseudonym_signature(
                        &payload.author_pseudonym.0,
                        &payload.signing_bytes(),
                        &sig_arr,
                    )
                    .is_err()
                    {
                        continue;
                    }
                    all_entries.push((payload.author_pseudonym, payload.entries));
                }
            }
            _ => {}
        }
    }
    Ok(all_entries)
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
        pseudonym_signing,
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

/// (segment_index, registry_key, slot_range_start) for each segment to try
/// in claim order. Segment 0 is the inviter's registry (`invite.registry_key`);
/// later segments come from the merged governance state's `SegmentAdded`
/// entries (architecture §15.2).
struct SegmentClaimCandidate {
    segment_index: u32,
    registry_key: String,
    slot_range_start: u32,
}

async fn claim_registry_slot(
    state: &Arc<AppState>,
    rc: &veilid_core::RoutingContext,
    invite_registry_key: &str,
    slot_seed_hex: &str,
    my_pseudo: &PseudonymKey,
    pseudonym_signing: &ed25519_dalek::SigningKey,
    segments: &[rekindle_governance::state::SegmentState],
) -> Result<(RecordKey, u32, u32, veilid_core::KeyPair), String> {
    let slot_seed_bytes: [u8; 32] = hex::decode(slot_seed_hex)
        .map_err(|e| format!("invalid slot seed hex: {e}"))?
        .try_into()
        .map_err(|_| "slot seed must be 32 bytes")?;

    // Build the candidate list: segment 0 from the invite, plus every
    // additional segment merged from governance. Sort ascending so we
    // prefer the lowest-indexed segment with space (deterministic +
    // backward-compatible with single-segment communities).
    let mut candidates: Vec<SegmentClaimCandidate> = Vec::new();
    candidates.push(SegmentClaimCandidate {
        segment_index: 0,
        registry_key: invite_registry_key.to_string(),
        slot_range_start: 0,
    });
    for seg in segments {
        if seg.segment_index == 0 {
            continue; // segment 0 is implicit
        }
        candidates.push(SegmentClaimCandidate {
            segment_index: seg.segment_index,
            registry_key: seg.registry_key.clone(),
            slot_range_start: seg.slot_range_start,
        });
    }
    candidates.sort_by_key(|c| c.segment_index);

    let mut last_full_segment: Option<u32> = None;
    for candidate in &candidates {
        let registry_typed_key = match candidate.registry_key.parse::<RecordKey>() {
            Ok(k) => k,
            Err(e) => {
                tracing::warn!(
                    segment = candidate.segment_index,
                    error = %e,
                    "claim_registry_slot: invalid registry key — skipping segment"
                );
                continue;
            }
        };

        let _reg_desc = match rc.open_dht_record(registry_typed_key.clone(), None).await {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!(
                    segment = candidate.segment_index,
                    error = %e,
                    "claim_registry_slot: failed to open segment registry — skipping"
                );
                continue;
            }
        };
        let report = rc
            .inspect_dht_record(
                registry_typed_key.clone(),
                Some(veilid_core::ValueSubkeyRangeSet::full()),
                veilid_core::DHTReportScope::UpdateGet,
            )
            .await
            .map_err(|e| format!("registry inspect failed (segment {}): {e}", candidate.segment_index))?;

        let Some(local_subkey) = (0..255u32).find(|subkey| {
            report
                .network_seqs()
                .get(*subkey as usize)
                .is_some_and(|seq| *seq == veilid_core::ValueSeqNum::default())
        }) else {
            last_full_segment = Some(candidate.segment_index);
            continue;
        };

        // Slot keypair derivation uses the GLOBAL slot index (architecture
        // §8.3 + §15.2 — slot 255 = local subkey 0 of segment 1, etc.).
        let global_slot = candidate.slot_range_start + local_subkey;
        let slot_kp = derive::derive_slot_keypair(&slot_seed_bytes, global_slot)
            .map_err(|e| format!("slot keypair derivation failed: {e}"))?;
        let slot_veilid = super::super::create::slot_signing_to_veilid(&slot_kp);
        let mut presence = MemberPresence {
            pseudonym_key: my_pseudo.clone(),
            display_name: Some(state_helpers::identity_display_name(state)),
            status: join_status_label(state).into(),
            route_blob: vec![],
            last_heartbeat: rekindle_utils::timestamp_secs(),
            ..Default::default()
        };
        let presence_sig =
            derive::sign_with_pseudonym(pseudonym_signing, &presence.signing_bytes());
        presence.signature = presence_sig.to_vec();
        let presence_bytes = serde_json::to_vec(&presence)
            .map_err(|e| format!("presence serialization failed: {e}"))?;
        rc.set_dht_value(
            registry_typed_key.clone(),
            local_subkey,
            presence_bytes,
            Some(SetDHTValueOptions {
                writer: Some(slot_veilid.clone()),
                ..Default::default()
            }),
        )
        .await
        .map_err(|e| format!("slot claim write failed: {e}"))?;

        let verify = rc
            .get_dht_value(registry_typed_key.clone(), local_subkey, true)
            .await
            .map_err(|e| format!("slot verification read failed: {e}"))?
            .ok_or("slot read-back returned empty after write")?;
        let written: MemberPresence = serde_json::from_slice(verify.data())
            .map_err(|e| format!("slot read-back deserialization failed: {e}"))?;
        if written.pseudonym_key != *my_pseudo {
            // Slot collision (architecture §6.4) — another joiner won
            // this slot. Try the next subkey in this same segment by
            // continuing the outer loop with a slot_range bump? Simpler:
            // surface the error so the caller can retry the join.
            return Err(format!(
                "Slot collision in segment {} — another member claimed this slot. Please retry.",
                candidate.segment_index
            ));
        }

        return Ok((registry_typed_key, candidate.segment_index, local_subkey, slot_veilid));
    }

    if let Some(seg) = last_full_segment {
        Err(format!(
            "Community is full — all 255 slots in segment {seg} occupied. Admin must call expand_community_segment to add a new segment."
        ))
    } else {
        Err("No reachable segment registry — Veilid attach may have failed".to_string())
    }
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
            recent_messages = bundle.recent_messages.len(),
            has_wrapped_owner_keypair = bundle.has_wrapped_owner_keypair,
            "bootstrap bundle received; warming peer state before DHT verification"
        );
        // Architecture §26 W26 — the bundle comes from the inviter and
        // the per-member entries are NOT individually signed (they're
        // the inviter's local view, not signed presence rows). A
        // malicious inviter could otherwise inject a bogus route_blob
        // for a real pseudonym and cause our outbound traffic to that
        // peer to land at attacker-controlled infrastructure. So we
        // accept the bundle's pseudonyms as `known_members` (display
        // hints only) but DO NOT promote them into `initial_online` —
        // the next-step DHT registry scan reads the actual signed
        // presence rows and is the only source for routing data.
        for member in &bundle.member_list {
            known_members.insert(member.pseudonym_key.clone());
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
                // Architecture §26 W26 — verify before merging.
                let Ok(sig_arr): Result<[u8; 64], _> =
                    presence.signature.as_slice().try_into()
                else {
                    continue;
                };
                if rekindle_secrets::derive::verify_pseudonym_signature(
                    &presence.pseudonym_key.0,
                    &presence.signing_bytes(),
                    &sig_arr,
                )
                .is_err()
                {
                    continue;
                }
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
