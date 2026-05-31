//! Community lifecycle operations — create, join, leave.
//!
//! Orchestration logic that composes:
//! - `broadcast::dht_writes` for raw DHT primitives (create_dflt, set, open, close, watch)
//! - `broadcast::route` for route allocation
//! - `dht/*` typed modules for business logic reads/writes (governance, registry, mailbox)
//!
//! The join flow is split into three phases for event-driven completion:
//! - `submit_join_request` — writes to inbox, returns immediately
//! - `await_join_approval` — select! between registry poll (tier 3) and direct notification (tier 2)
//! - `complete_join` — reads channels + MEKs after approval confirmed
//!
//! `join_community` is a convenience wrapper that calls all three sequentially.

use std::sync::Arc;

use parking_lot::RwLock;
use tracing::info;

use crate::broadcast::node::TransportNode;
use crate::crypto::mek::{Mek, MekCache};
use crate::error::{Result, TransportError};
use crate::payload::dht_types::{
    ChannelEntry, ChannelKind, CommunityMetadata, JoinPolicy, MemberSummary, PendingJoinEntry,
    PendingJoinStatus,
};
use crate::payload::rpc::ChannelEntrySummary;
use crate::session::Session;

pub struct CommunityCreated {
    pub governance_key: String,
    pub governance_keypair_bytes: Vec<u8>,
    pub registry_key: String,
    pub registry_keypair_bytes: Vec<u8>,
    pub community_mailbox_key: String,
    pub join_inbox_key: String,
    pub default_channel_id: String,
    pub our_pseudonym_key: String,
    pub our_slot_index: u32,
    pub mek_generation: u64,
}

/// Returned by `submit_join_request` — metadata needed for approval await + completion.
pub struct JoinRequestSubmitted {
    pub community_name: String,
    pub governance_key: String,
    pub our_pseudonym_hex: String,
    pub registry_key: String,
    pub community_mailbox_key: String,
}

pub struct JoinResult {
    pub community_name: String,
    pub governance_key: String,
    pub our_pseudonym_key: String,
    pub display_name: String,
    pub our_slot_index: u32,
    pub registry_key: String,
    pub community_mailbox_key: String,
    pub channels: Vec<ChannelEntrySummary>,
    pub meks_cached: usize,
    pub slot_seed: [u8; 32],
}

pub struct LeaveResult {
    pub leave_payload_bytes: Vec<u8>,
}

pub async fn create_community(
    node: &TransportNode,
    session: &Session,
    name: &str,
    description: Option<&str>,
    mek_cache: &Arc<RwLock<MekCache>>,
    signing_key_bytes: &[u8; 32],
) -> Result<CommunityCreated> {
    info!(name, "creating community");
    let dht = node.dht()?;

    // Step 1: Create governance manifest (typed business logic)
    let initial_metadata = CommunityMetadata {
        name: name.to_string(),
        description: description.map(String::from),
        icon_hash: None,
        banner_hash: None,
        created_at: rekindle_utils::timestamp_ms(),
        owner_pseudonym: session.identity.public_key_hex.clone(),
        last_refreshed: rekindle_utils::timestamp_ms(),
        join_policy: JoinPolicy::AutoAllow,
        community_mailbox_key: String::new(),
        operator_pseudonyms: vec![session.identity.public_key_hex.clone()],
        max_members: crate::payload::dht_types::REGISTRY_MAX_MEMBERS,
        mek_rotation_interval_hours: 168,
        join_inbox_key: String::new(),
        join_inbox_keypair_hex: String::new(),
    };
    let (governance_key, governance_keypair) = dht
        .governance()
        .create(&initial_metadata)
        .await
        .map_err(|e| TransportError::CommunityCreationFailed {
            reason: format!("governance manifest: {e}"),
        })?;
    let governance_keypair_bytes = governance_keypair
        .map(|kp| super::identity::serialize_keypair(&kp))
        .unwrap_or_default();
    info!(key = %governance_key, "governance manifest created");

    // Step 2: Create community mailbox (typed business logic)
    let gov_kp = crate::broadcast::node::deserialize_keypair(&governance_keypair_bytes)?;
    let community_mailbox_key = dht
        .mailbox()
        .create_community_mailbox(gov_kp.clone())
        .await
        .map_err(|e| TransportError::CommunityCreationFailed {
            reason: format!("community mailbox: {e}"),
        })?;
    info!(key = %community_mailbox_key, "community mailbox created");

    // Step 3: Allocate community route (broadcast primitive), publish to mailbox (typed)
    let (route_id, route_blob) = crate::broadcast::route::allocate_community(node)
        .await
        .map_err(|e| TransportError::CommunityCreationFailed {
            reason: format!("community route: {e}"),
        })?;
    dht.mailbox()
        .update_community_route(&community_mailbox_key, &route_blob)
        .await
        .map_err(|e| TransportError::CommunityCreationFailed {
            reason: format!("mailbox route publish: {e}"),
        })?;
    info!(route = route_id, "community route published to mailbox");

    // Step 4: Create member registry (typed business logic)
    let (registry_key, registry_keypair) =
        dht.registry()
            .create()
            .await
            .map_err(|e| TransportError::CommunityCreationFailed {
                reason: format!("member registry: {e}"),
            })?;
    let registry_keypair_bytes = registry_keypair
        .map(|kp| super::identity::serialize_keypair(&kp))
        .unwrap_or_default();
    info!(key = %registry_key, "member registry created");

    // Step 5: Create join inbox (raw primitive — DFLT with JOIN_INBOX_SUBKEY_COUNT subkeys)
    //
    // SECURITY NOTE (v1 limitation): The join inbox uses a DFLT record with a
    // published keypair. Any node that reads the governance metadata can write to
    // any inbox subkey, potentially overwriting legitimate join requests. Mitigation:
    // each PendingJoinEntry includes an Ed25519 signature over the entry content,
    // verified by process_inbox before approval. A SMPL-based inbox with per-joiner
    // writer slots is planned for v2 to eliminate the overwrite vector entirely.
    #[allow(clippy::cast_possible_truncation)]
    let (inbox_key, inbox_keypair) = crate::broadcast::dht_writes::create_dflt(
        node,
        crate::payload::dht_types::JOIN_INBOX_SUBKEY_COUNT as u16,
        None,
    )
    .await
    .map_err(|e| TransportError::CommunityCreationFailed {
        reason: format!("join inbox: {e}"),
    })?;
    let inbox_keypair_hex = inbox_keypair
        .map(|kp| hex::encode(super::identity::serialize_keypair(&kp)))
        .unwrap_or_default();
    crate::broadcast::dht_writes::set(node, &inbox_key, 0, b"[]".to_vec(), None)
        .await
        .map_err(|e| TransportError::CommunityCreationFailed {
            reason: format!("inbox seed: {e}"),
        })?;
    let inbox_subkeys: Vec<u32> = (0..crate::payload::dht_types::JOIN_INBOX_SUBKEY_COUNT).collect();
    let _ = crate::broadcast::dht_writes::watch(node, &inbox_key, &inbox_subkeys).await;
    info!(key = %inbox_key, "join inbox created, seeded, and watched");

    // Step 6: Create default #general channel (typed governance write)
    let channel_id = uuid::Uuid::new_v4().to_string();
    let channel_entry = ChannelEntry {
        id: channel_id.clone(),
        name: "general".to_string(),
        kind: ChannelKind::Text,
        sort_order: 0,
        category_id: None,
        topic: "General discussion".to_string(),
        slowmode_seconds: 0,
        nsfw: false,
        message_record_key: None,
        mek_generation: 1,
        log_key: None,
    };
    dht.governance()
        .write_channels(&governance_key, &[channel_entry])
        .await
        .map_err(|e| TransportError::CommunityCreationFailed {
            reason: format!("channel directory: {e}"),
        })?;

    // Step 7: Generate MEK, wrap to creator, publish to vault (typed registry write)
    let mek = Mek::generate(1);
    let mek_wire = mek.to_wire_bytes();
    mek_cache.write().insert(&governance_key, &channel_id, mek);
    let creator_pseudonym =
        crate::crypto::pseudonym::derive_community_pseudonym(signing_key_bytes, &governance_key);
    let creator_pseudonym_pub = creator_pseudonym.verifying_key().to_bytes();
    let creator_pseudonym_hex = hex::encode(creator_pseudonym_pub);
    let wrapped_mek =
        crate::crypto::mek::wrap_mek(&creator_pseudonym, &creator_pseudonym_pub, &mek_wire)
            .map_err(|e| TransportError::CommunityCreationFailed {
                reason: format!("mek wrap: {e}"),
            })?;
    let vault_entry = crate::payload::dht_types::MekVaultEntry {
        channel_id: channel_id.clone(),
        generation: 1,
        rotator_pseudonym: creator_pseudonym_hex.clone(),
        copies: vec![crate::payload::dht_types::EncryptedMekCopy {
            target_pseudonym: creator_pseudonym_hex,
            encrypted_mek: wrapped_mek,
        }],
    };
    dht.registry()
        .write_mek_vault(&registry_key, &[vault_entry])
        .await
        .map_err(|e| TransportError::CommunityCreationFailed {
            reason: format!("mek vault: {e}"),
        })?;
    info!("initial MEK published to vault");

    // Step 8: Register creator as owner (typed registry write)
    let our_pseudonym_key = session.identity.public_key_hex.clone();
    let owner_member = MemberSummary {
        pseudonym_key: our_pseudonym_key.clone(),
        display_name: session.identity.display_name.clone(),
        role_ids: vec![0],
        joined_at: rekindle_utils::timestamp_ms(),
        subkey_index: 0,
        onboarding_complete: true,
        timeout_until: None,
        profile_dht_key: Some(session.identity.profile_dht_key.clone()),
        channel_records: std::collections::HashMap::new(),
    };
    dht.registry()
        .write_member_index(&registry_key, &[owner_member])
        .await
        .map_err(|e| TransportError::CommunityCreationFailed {
            reason: format!("owner registration: {e}"),
        })?;

    // Step 9: Write final metadata (typed governance write)
    let final_metadata = CommunityMetadata {
        community_mailbox_key: community_mailbox_key.clone(),
        join_inbox_key: inbox_key.clone(),
        join_inbox_keypair_hex: inbox_keypair_hex,
        ..initial_metadata
    };
    dht.governance()
        .write_metadata(&governance_key, &final_metadata)
        .await
        .map_err(|e| TransportError::CommunityCreationFailed {
            reason: format!("metadata update: {e}"),
        })?;

    // Step 10: Write registry spine (raw primitive — custom subkey)
    let spine = serde_json::json!({ "primary_key": registry_key, "segments": [] });
    let spine_bytes =
        serde_json::to_vec(&spine).map_err(|e| TransportError::SerializationFailed {
            reason: e.to_string(),
        })?;
    crate::broadcast::dht_writes::set(
        node,
        &governance_key,
        crate::payload::dht_types::MANIFEST_REGISTRY_SPINE,
        spine_bytes,
        None,
    )
    .await
    .map_err(|e| TransportError::CommunityCreationFailed {
        reason: format!("registry spine: {e}"),
    })?;

    info!(governance = %governance_key, registry = %registry_key, mailbox = %community_mailbox_key, inbox = %inbox_key, "community created — verifying network propagation");

    // Step 11: Verify critical records are network-readable (write-then-verify).
    // Other nodes depend on these records. If they can't read them, joins will fail.
    // Retry with exponential backoff — propagation typically takes 1-10 seconds.
    let verify_deadline = std::time::Duration::from_secs(30);
    let verify_start = std::time::Instant::now();
    let mut backoff = std::time::Duration::from_millis(500);
    let ceiling = std::time::Duration::from_secs(5);

    loop {
        // Read governance metadata with force_refresh — verify join_inbox_key is visible
        let metadata_ok = match crate::broadcast::dht_writes::get(
            node,
            &governance_key,
            crate::payload::dht_types::MANIFEST_METADATA,
            true,
        )
        .await
        {
            Ok(Some(data)) => serde_json::from_slice::<CommunityMetadata>(&data)
                .map(|m| !m.join_inbox_key.is_empty())
                .unwrap_or(false),
            _ => false,
        };

        if metadata_ok {
            let elapsed = verify_start.elapsed().as_millis();
            info!(governance = %governance_key, elapsed_ms = elapsed, "community verified — metadata propagated with join_inbox_key");
            break;
        }

        if verify_start.elapsed() >= verify_deadline {
            tracing::warn!(
                governance = %governance_key,
                "community created but metadata propagation not confirmed within 30s — \
                 joiners may experience initial delay"
            );
            break;
        }

        info!(
            governance = %governance_key,
            elapsed_secs = verify_start.elapsed().as_secs(),
            backoff_ms = backoff.as_millis(),
            "community verify: metadata not yet propagated, retrying"
        );
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(ceiling);
    }

    Ok(CommunityCreated {
        governance_key,
        governance_keypair_bytes,
        registry_key,
        registry_keypair_bytes,
        community_mailbox_key,
        join_inbox_key: inbox_key,
        default_channel_id: channel_id,
        our_pseudonym_key,
        our_slot_index: 0,
        mek_generation: 1,
    })
}

// ── Join: Phase 1 — Submit request to DHT inbox ─────────────────────────

/// Submit a join request to a community's DHT inbox. Returns immediately.
///
/// This is the non-blocking first phase of the join flow. After this returns,
/// the caller should await approval via `await_join_approval`.
pub async fn submit_join_request(
    node: &TransportNode,
    session: &Session,
    governance_key: &str,
    display_name: &str,
    signing_key_bytes: &[u8; 32],
) -> Result<JoinRequestSubmitted> {
    info!(governance = governance_key, "joining community via DHT");
    let dht = node.dht()?;

    // Step 1: Open governance readonly, read metadata
    crate::broadcast::dht_writes::open_readonly(node, governance_key)
        .await
        .map_err(|e| TransportError::JoinRejected {
            community: governance_key.to_string(),
            reason: format!("cannot open governance: {e}"),
        })?;
    let metadata = dht
        .governance()
        .read_metadata(governance_key)
        .await?
        .ok_or_else(|| TransportError::JoinRejected {
            community: governance_key.to_string(),
            reason: "governance metadata not found".into(),
        })?;
    info!(community = %metadata.name, "governance metadata read");

    // Step 2: Validate inbox, retry with force_refresh
    let metadata =
        if metadata.join_inbox_key.is_empty() || metadata.join_inbox_keypair_hex.is_empty() {
            info!("inbox key empty, fetching fresh from network");
            match crate::broadcast::dht_writes::get(
                node,
                governance_key,
                crate::payload::dht_types::MANIFEST_METADATA,
                true,
            )
            .await?
            {
                Some(data) => serde_json::from_slice::<CommunityMetadata>(&data).map_err(|e| {
                    TransportError::JoinRejected {
                        community: governance_key.to_string(),
                        reason: format!("metadata parse: {e}"),
                    }
                })?,
                None => {
                    return Err(TransportError::JoinRejected {
                        community: governance_key.to_string(),
                        reason: "metadata not found after refresh".into(),
                    })
                }
            }
        } else {
            metadata
        };
    if metadata.join_inbox_key.is_empty() || metadata.join_inbox_keypair_hex.is_empty() {
        return Err(TransportError::JoinRejected {
            community: metadata.name.clone(),
            reason: "no join inbox".into(),
        });
    }

    // Step 3: Derive pseudonym
    let pseudonym =
        crate::crypto::pseudonym::derive_community_pseudonym(signing_key_bytes, governance_key);
    let our_pseudonym_hex = hex::encode(pseudonym.verifying_key().to_bytes());

    // Step 4: Write join request to inbox
    let inbox_kp_bytes = hex::decode(&metadata.join_inbox_keypair_hex).map_err(|e| {
        TransportError::JoinRejected {
            community: metadata.name.clone(),
            reason: format!("invalid inbox keypair: {e}"),
        }
    })?;
    let inbox_kp = crate::broadcast::node::deserialize_keypair(&inbox_kp_bytes)?;
    crate::broadcast::dht_writes::open_writable(node, &metadata.join_inbox_key, inbox_kp)
        .await
        .map_err(|e| TransportError::JoinRejected {
            community: metadata.name.clone(),
            reason: format!("cannot open inbox: {e}"),
        })?;
    let subkey_index = pseudonym_to_inbox_subkey(&our_pseudonym_hex);
    let mut join_entry = PendingJoinEntry {
        requester_pseudonym_hex: our_pseudonym_hex.clone(),
        display_name: display_name.to_string(),
        profile_dht_key: session.identity.profile_dht_key.clone(),
        invite_code_hash: None,
        requested_at: rekindle_utils::timestamp_ms(),
        status: PendingJoinStatus::Pending,
        signature_hex: String::new(),
    };
    // Sign with pseudonym key to prevent impersonation (see Finding 3)
    let content = join_entry.signature_content();
    use ed25519_dalek::Signer;
    let signature = pseudonym.sign(&content);
    join_entry.signature_hex = hex::encode(signature.to_bytes());
    // Read-append-write: read existing entries at this subkey, append ours
    let existing =
        match crate::broadcast::dht_writes::get(node, &metadata.join_inbox_key, subkey_index, true)
            .await
        {
            Ok(Some(data)) if !data.is_empty() && data != b"[]" => data,
            _ => Vec::new(),
        };
    let mut entries: Vec<PendingJoinEntry> = if existing.is_empty() {
        Vec::new()
    } else {
        serde_json::from_slice::<Vec<PendingJoinEntry>>(&existing)
            .or_else(|_| serde_json::from_slice::<PendingJoinEntry>(&existing).map(|e| vec![e]))
            .unwrap_or_default()
    };
    entries.retain(|e| e.requester_pseudonym_hex != join_entry.requester_pseudonym_hex);
    entries.push(join_entry);
    let entry_bytes =
        serde_json::to_vec(&entries).map_err(|e| TransportError::SerializationFailed {
            reason: e.to_string(),
        })?;
    crate::broadcast::dht_writes::set(
        node,
        &metadata.join_inbox_key,
        subkey_index,
        entry_bytes,
        None,
    )
    .await
    .map_err(|e| TransportError::JoinRejected {
        community: metadata.name.clone(),
        reason: format!("inbox write: {e}"),
    })?;
    info!(community = %metadata.name, subkey = subkey_index, "join request written");

    // Step 5: Read registry key from spine
    let registry_key = read_registry_key(node, governance_key, &metadata.name).await?;

    Ok(JoinRequestSubmitted {
        community_name: metadata.name,
        governance_key: governance_key.to_string(),
        our_pseudonym_hex,
        registry_key,
        community_mailbox_key: metadata.community_mailbox_key,
    })
}

// ── Join: Phase 2 — Await approval via tier 2 (direct) + tier 3 (poll) ──

/// Await community join approval. Uses select! between:
/// - Tier 2: direct notification via oneshot (instant when operator sends JoinAccepted)
/// - Tier 3: registry poll with force_refresh (fallback, 2s interval)
///
/// Returns the assigned slot_index on success.
pub async fn await_join_approval(
    node: &TransportNode,
    registry_key: &str,
    our_pseudonym_hex: &str,
    community_name: &str,
    mut notify_rx: Option<tokio::sync::oneshot::Receiver<u32>>,
    timeout_secs: u64,
) -> Result<u32> {
    let poll_interval = std::time::Duration::from_secs(2);
    let deadline = std::time::Duration::from_secs(timeout_secs);
    let start = std::time::Instant::now();

    info!(
        community = community_name,
        timeout_secs, "awaiting join approval"
    );

    loop {
        if start.elapsed() >= deadline {
            return Err(TransportError::JoinRejected {
                community: community_name.to_string(),
                reason: format!(
                    "not approved within {timeout_secs}s — the community operator may be offline \
                     or DHT propagation is slow. Try again later."
                ),
            });
        }

        tokio::select! {
            // Tier 2: direct notification from operator (instant)
            result = async {
                match notify_rx.as_mut() {
                    Some(rx) => rx.await,
                    None => std::future::pending().await,
                }
            } => {
                match result {
                    Ok(slot_index) => {
                        info!(community = community_name, slot = slot_index, "approved via direct notification (tier 2)");
                        return Ok(slot_index);
                    }
                    Err(_) => {
                        // Sender dropped (e.g., handler shutdown) — fall through to poll only
                        notify_rx = None;
                    }
                }
            }
            // Tier 3: registry poll with force_refresh
            () = tokio::time::sleep(poll_interval) => {
                let elapsed = start.elapsed().as_secs();
                let open_result = crate::broadcast::dht_writes::open_readonly(node, registry_key).await;
                if let Err(ref e) = open_result {
                    info!(registry_key, elapsed, error = %e, "join poll: registry open failed");
                }
                let members: Vec<MemberSummary> = match crate::broadcast::dht_writes::get(
                    node, registry_key, crate::payload::dht_types::REGISTRY_MEMBER_INDEX, true,
                ).await {
                    Ok(Some(data)) => {
                        let parsed: Vec<MemberSummary> = serde_json::from_slice(&data).unwrap_or_default();
                        info!(
                            community = community_name, elapsed,
                            member_count = parsed.len(),
                            members = ?parsed.iter().map(|m| &m.pseudonym_key[..16.min(m.pseudonym_key.len())]).collect::<Vec<_>>(),
                            "join poll: registry read"
                        );
                        parsed
                    }
                    Ok(None) => {
                        info!(community = community_name, elapsed, "join poll: registry subkey empty");
                        Vec::new()
                    }
                    Err(e) => {
                        info!(community = community_name, elapsed, error = %e, "join poll: registry read failed");
                        Vec::new()
                    }
                };
                if let Some(m) = members.iter().find(|m| m.pseudonym_key == our_pseudonym_hex) {
                    info!(community = community_name, slot = m.subkey_index, elapsed, "approved via registry poll (tier 3)");
                    return Ok(m.subkey_index);
                }
            }
        }
    }
}

// ── Join: Phase 3 — Complete join (read channels + MEKs) ────────────────

/// Complete the join after approval: read channels and cache MEKs.
pub async fn complete_join(
    node: &TransportNode,
    submitted: &JoinRequestSubmitted,
    slot_index: u32,
    mek_cache: &Arc<RwLock<MekCache>>,
    signing_key_bytes: &[u8; 32],
) -> Result<JoinResult> {
    let dht = node.dht()?;

    // Read channels (typed governance read)
    let channels = dht
        .governance()
        .read_channels(&submitted.governance_key)
        .await
        .unwrap_or_default();
    let channel_summaries: Vec<ChannelEntrySummary> = channels
        .iter()
        .map(|ch| ChannelEntrySummary {
            id: ch.id.clone(),
            name: ch.name.clone(),
            kind: format!("{:?}", ch.kind).to_lowercase(),
            mek_generation: ch.mek_generation,
        })
        .collect();

    // Read MEK vault (typed registry read)
    // The vault may not have propagated yet if the operator just wrote it.
    // Try cached read first, then force_refresh if no copies found for us.
    let mut meks_cached = 0usize;
    let mut vault = dht
        .registry()
        .read_mek_vault(&submitted.registry_key)
        .await
        .unwrap_or_default();

    // Tier 3 fallback: if no copies for our pseudonym, retry with force_refresh
    if !vault.iter().any(|e| {
        e.copies
            .iter()
            .any(|c| c.target_pseudonym == submitted.our_pseudonym_hex)
    }) {
        info!(community = %submitted.community_name, "MEK vault has no copies for us — retrying with force_refresh");
        let mut backoff = std::time::Duration::from_secs(2);
        let deadline = std::time::Duration::from_secs(20);
        let start = std::time::Instant::now();
        while start.elapsed() < deadline {
            tokio::time::sleep(backoff).await;
            if let Ok(Some(data)) = crate::broadcast::dht_writes::get(
                node,
                &submitted.registry_key,
                crate::payload::dht_types::REGISTRY_MEK_VAULT,
                true,
            )
            .await
            {
                let fresh: Vec<crate::payload::dht_types::MekVaultEntry> =
                    serde_json::from_slice(&data).unwrap_or_default();
                if fresh.iter().any(|e| {
                    e.copies
                        .iter()
                        .any(|c| c.target_pseudonym == submitted.our_pseudonym_hex)
                }) {
                    vault = fresh;
                    info!(community = %submitted.community_name, elapsed_ms = start.elapsed().as_millis(), "MEK vault propagated — copies found");
                    break;
                }
            }
            backoff = (backoff * 2).min(std::time::Duration::from_secs(5));
        }
    }

    for entry in &vault {
        if let Some(copy) = entry
            .copies
            .iter()
            .find(|c| c.target_pseudonym == submitted.our_pseudonym_hex)
        {
            let transfer = crate::payload::rpc::MekTransferPayload {
                channel_id: entry.channel_id.clone(),
                generation: entry.generation,
                rotator_pseudonym_hex: entry.rotator_pseudonym.clone(),
                wrapped_mek: copy.encrypted_mek.clone(),
            };
            match super::mek::receive_mek_transfer_payload(
                &transfer,
                signing_key_bytes,
                &submitted.governance_key,
                mek_cache,
            ) {
                Ok(_) => {
                    meks_cached += 1;
                }
                Err(e) => {
                    tracing::warn!(channel = %entry.channel_id, error = %e, "MEK vault unwrap failed");
                }
            }
        }
    }
    info!(community = %submitted.community_name, meks_cached, "MEKs cached");

    let slot_seed = derive_slot_seed(signing_key_bytes, &submitted.governance_key, slot_index);
    Ok(JoinResult {
        community_name: submitted.community_name.clone(),
        governance_key: submitted.governance_key.clone(),
        our_pseudonym_key: submitted.our_pseudonym_hex.clone(),
        display_name: String::new(), // caller fills from session
        our_slot_index: slot_index,
        registry_key: submitted.registry_key.clone(),
        community_mailbox_key: submitted.community_mailbox_key.clone(),
        channels: channel_summaries,
        meks_cached,
        slot_seed,
    })
}

// ── Convenience wrapper (backward compat) ───────────────────────────────

/// Join a community end-to-end. Convenience wrapper that calls all three phases
/// sequentially without a direct notification channel (tier 3 poll only).
///
/// The daemon's `handle_join` uses the split functions directly with a tier 2
/// notification channel for faster completion.
pub async fn join_community(
    node: &TransportNode,
    session: &Session,
    governance_key: &str,
    display_name: &str,
    mek_cache: &Arc<RwLock<MekCache>>,
    signing_key_bytes: &[u8; 32],
) -> Result<JoinResult> {
    let submitted = submit_join_request(
        node,
        session,
        governance_key,
        display_name,
        signing_key_bytes,
    )
    .await?;
    let slot_index = await_join_approval(
        node,
        &submitted.registry_key,
        &submitted.our_pseudonym_hex,
        &submitted.community_name,
        None,
        120,
    )
    .await?;
    let mut result =
        complete_join(node, &submitted, slot_index, mek_cache, signing_key_bytes).await?;
    result.display_name = display_name.to_string();
    Ok(result)
}

// ── Leave ───────────────────────────────────────────────────────────────

pub async fn leave_community(
    node: &TransportNode,
    membership: &crate::session::CommunityMembership,
    mek_cache: &Arc<RwLock<MekCache>>,
    signing_key_bytes: &[u8; 32],
) -> Result<LeaveResult> {
    info!(community = %membership.community_name, "leaving community via DHT");
    let dht = node.dht()?;

    let _ = crate::broadcast::dht_writes::open_readonly(node, &membership.governance_key).await;
    let metadata = dht
        .governance()
        .read_metadata(&membership.governance_key)
        .await?
        .ok_or_else(|| TransportError::Internal("governance metadata not found".into()))?;

    if !metadata.join_inbox_key.is_empty() && !metadata.join_inbox_keypair_hex.is_empty() {
        if let Ok(inbox_kp_bytes) = hex::decode(&metadata.join_inbox_keypair_hex) {
            if let Ok(inbox_kp) = crate::broadcast::node::deserialize_keypair(&inbox_kp_bytes) {
                let _ = crate::broadcast::dht_writes::open_writable(
                    node,
                    &metadata.join_inbox_key,
                    inbox_kp,
                )
                .await;
                let pseudonym = crate::crypto::pseudonym::derive_community_pseudonym(
                    signing_key_bytes,
                    &membership.governance_key,
                );
                let our_pseudonym_hex = hex::encode(pseudonym.verifying_key().to_bytes());
                let mut leave_entry = PendingJoinEntry {
                    requester_pseudonym_hex: our_pseudonym_hex.clone(),
                    display_name: membership.display_name.clone(),
                    profile_dht_key: String::new(),
                    invite_code_hash: None,
                    requested_at: rekindle_utils::timestamp_ms(),
                    status: PendingJoinStatus::Left {
                        left_at: rekindle_utils::timestamp_ms(),
                    },
                    signature_hex: String::new(),
                };
                let content = leave_entry.signature_content();
                use ed25519_dalek::Signer;
                let sig = pseudonym.sign(&content);
                leave_entry.signature_hex = hex::encode(sig.to_bytes());
                let subkey = pseudonym_to_inbox_subkey(&our_pseudonym_hex);
                // Read-append-write: preserve existing entries at this subkey
                let existing = match crate::broadcast::dht_writes::get(
                    node,
                    &metadata.join_inbox_key,
                    subkey,
                    true,
                )
                .await
                {
                    Ok(Some(data)) if !data.is_empty() && data != b"[]" => data,
                    _ => Vec::new(),
                };
                let mut inbox_entries: Vec<PendingJoinEntry> = if existing.is_empty() {
                    Vec::new()
                } else {
                    serde_json::from_slice::<Vec<PendingJoinEntry>>(&existing)
                        .or_else(|_| {
                            serde_json::from_slice::<PendingJoinEntry>(&existing).map(|e| vec![e])
                        })
                        .unwrap_or_default()
                };
                inbox_entries
                    .retain(|e| e.requester_pseudonym_hex != leave_entry.requester_pseudonym_hex);
                inbox_entries.push(leave_entry);
                let entry_bytes = serde_json::to_vec(&inbox_entries).map_err(|e| {
                    TransportError::SerializationFailed {
                        reason: e.to_string(),
                    }
                })?;
                let _ = crate::broadcast::dht_writes::set(
                    node,
                    &metadata.join_inbox_key,
                    subkey,
                    entry_bytes,
                    None,
                )
                .await;
                info!(community = %membership.community_name, "leave entry written");
            }
        }
    }

    mek_cache
        .write()
        .remove_community(&membership.governance_key);
    let gossip_payload = crate::payload::gossip::GossipPayload::Control(
        crate::payload::gossip::ControlPayload::MemberLeave {
            pseudonym_key: membership.pseudonym_key.clone(),
        },
    );
    let leave_payload_bytes =
        postcard::to_stdvec(&gossip_payload).map_err(|e| TransportError::SerializationFailed {
            reason: e.to_string(),
        })?;
    info!(community = %membership.community_name, "community left");
    Ok(LeaveResult {
        leave_payload_bytes,
    })
}

// ── Utilities ───────────────────────────────────────────────────────────

pub async fn read_inbox_requests(
    dht: &crate::broadcast::dht::DhtStore,
    inbox_key: &str,
) -> Result<Vec<PendingJoinEntry>> {
    let start = std::time::Instant::now();

    // Step 1: Inspect to find populated subkeys (one network call)
    let all_subkeys: Vec<u32> = (0..crate::payload::dht_types::JOIN_INBOX_SUBKEY_COUNT).collect();
    let populated = match crate::broadcast::dht::record::inspect(
        dht.routing_context(),
        inbox_key,
        Some(&all_subkeys),
    )
    .await
    {
        Ok(report) => {
            let pop: Vec<u32> = report
                .subkeys()
                .iter()
                .zip(report.local_seqs().iter())
                .filter(|(_, seq)| seq.is_some())
                .map(|(sk, _)| sk)
                .collect();
            info!(
                populated = pop.len(),
                total = crate::payload::dht_types::JOIN_INBOX_SUBKEY_COUNT,
                skipped = crate::payload::dht_types::JOIN_INBOX_SUBKEY_COUNT as usize - pop.len(),
                inspect_ms = start.elapsed().as_millis(),
                "read_inbox_requests: inspect found {} populated subkeys",
                pop.len()
            );
            pop
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                "read_inbox_requests: inspect failed, falling back to full scan (SLOW)"
            );
            all_subkeys
        }
    };

    if populated.is_empty() {
        info!(
            elapsed_ms = start.elapsed().as_millis(),
            "read_inbox_requests: inbox empty (no populated subkeys)"
        );
        return Ok(Vec::new());
    }

    // Step 2: Read only populated subkeys
    let mut entries = Vec::new();
    let mut read_errors = 0u32;
    for subkey in &populated {
        let data = match tokio::time::timeout(
            std::time::Duration::from_millis(5000),
            crate::broadcast::dht::record::get(dht.routing_context(), inbox_key, *subkey, false),
        )
        .await
        {
            Ok(Ok(Some(d))) if !d.is_empty() && d != b"[]" => d,
            Ok(Err(e)) => {
                tracing::debug!(subkey, error = %e, "read_inbox_requests: get failed");
                read_errors += 1;
                continue;
            }
            _ => continue,
        };
        // Parse as array first, fall back to single entry for backward compatibility
        let parsed: Vec<PendingJoinEntry> =
            match serde_json::from_slice::<Vec<PendingJoinEntry>>(&data) {
                Ok(arr) => arr,
                Err(_) => match serde_json::from_slice::<PendingJoinEntry>(&data) {
                    Ok(single) => vec![single],
                    Err(e) => {
                        tracing::debug!(subkey, error = %e, "read_inbox_requests: parse failed");
                        continue;
                    }
                },
            };
        for entry in parsed {
            if matches!(
                entry.status,
                PendingJoinStatus::Pending | PendingJoinStatus::Left { .. }
            ) {
                info!(
                    requester = %entry.display_name,
                    subkey,
                    status = ?entry.status,
                    "read_inbox_requests: found entry"
                );
                entries.push(entry);
            } else {
                tracing::debug!(subkey, status = ?entry.status, "read_inbox_requests: skipping non-actionable entry");
            }
        }
    }

    info!(
        entries = entries.len(),
        subkeys_read = populated.len(),
        read_errors,
        elapsed_ms = start.elapsed().as_millis(),
        "read_inbox_requests: complete"
    );
    Ok(entries)
}

fn pseudonym_to_inbox_subkey(pseudonym_hex: &str) -> u32 {
    let hash = blake3::hash(pseudonym_hex.as_bytes());
    let bytes = hash.as_bytes();
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
        % crate::payload::dht_types::JOIN_INBOX_SUBKEY_COUNT
}

async fn read_registry_key(
    node: &TransportNode,
    governance_key: &str,
    community_name: &str,
) -> Result<String> {
    match crate::broadcast::dht_writes::get(
        node,
        governance_key,
        crate::payload::dht_types::MANIFEST_REGISTRY_SPINE,
        false,
    )
    .await?
    {
        Some(data) if !data.is_empty() => {
            let spine: serde_json::Value =
                serde_json::from_slice(&data).map_err(|e| TransportError::JoinRejected {
                    community: community_name.to_string(),
                    reason: format!("spine parse: {e}"),
                })?;
            spine
                .get("primary_key")
                .and_then(serde_json::Value::as_str)
                .map(String::from)
                .ok_or_else(|| TransportError::JoinRejected {
                    community: community_name.to_string(),
                    reason: "spine missing primary_key".into(),
                })
        }
        _ => Err(TransportError::JoinRejected {
            community: community_name.to_string(),
            reason: "no registry spine".into(),
        }),
    }
}

fn derive_slot_seed(
    signing_key_bytes: &[u8; 32],
    governance_key: &str,
    slot_index: u32,
) -> [u8; 32] {
    let hkdf = hkdf::Hkdf::<sha2::Sha256>::new(Some(b"rekindle-slot-seed-v1"), signing_key_bytes);
    let mut info = Vec::with_capacity(governance_key.len() + 4);
    info.extend_from_slice(governance_key.as_bytes());
    info.extend_from_slice(&slot_index.to_le_bytes());
    let mut seed = [0u8; 32];
    hkdf.expand(&info, &mut seed).expect("32-byte HKDF output");
    seed
}
