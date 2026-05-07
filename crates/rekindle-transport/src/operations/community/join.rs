//! Community join — three-phase: submit request, await approval, complete.

use std::sync::Arc;

use parking_lot::RwLock;
use tracing::info;

use crate::crypto::mek::MekCache;
use crate::error::{TransportError, Result};
use crate::broadcast::node::TransportNode;
use crate::payload::dht_types::{
    CommunityMetadata, MemberSummary, PendingJoinEntry, PendingJoinStatus,
};
use crate::payload::rpc::ChannelEntrySummary;
use crate::session::Session;

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

// ── Phase 1: Submit request ───────────────────────────────────────────

pub async fn submit_join_request(
    node: &TransportNode, session: &Session, governance_key: &str,
    display_name: &str, signing_key_bytes: &[u8; 32],
) -> Result<JoinRequestSubmitted> {
    info!(governance = governance_key, "joining community via DHT");
    let dht = node.dht()?;

    crate::broadcast::dht_writes::open_readonly(node, governance_key).await
        .map_err(|e| TransportError::JoinRejected { community: governance_key.to_string(), reason: format!("cannot open governance: {e}") })?;
    let metadata = dht.governance().read_metadata(governance_key).await?
        .ok_or_else(|| TransportError::JoinRejected { community: governance_key.to_string(), reason: "governance metadata not found".into() })?;
    info!(community = %metadata.name, "governance metadata read");

    // Validate inbox, retry with force_refresh
    let metadata = if metadata.join_inbox_key.is_empty() || metadata.join_inbox_keypair_hex.is_empty() {
        info!("inbox key empty, fetching fresh from network");
        match crate::broadcast::dht_writes::get(node, governance_key, crate::payload::dht_types::MANIFEST_METADATA, true).await? {
            Some(data) => serde_json::from_slice::<CommunityMetadata>(&data)
                .map_err(|e| TransportError::JoinRejected { community: governance_key.to_string(), reason: format!("metadata parse: {e}") })?,
            None => return Err(TransportError::JoinRejected { community: governance_key.to_string(), reason: "metadata not found after refresh".into() }),
        }
    } else { metadata };
    if metadata.join_inbox_key.is_empty() || metadata.join_inbox_keypair_hex.is_empty() {
        return Err(TransportError::JoinRejected { community: metadata.name.clone(), reason: "no join inbox".into() });
    }

    // Derive pseudonym
    let pseudonym = crate::crypto::pseudonym::derive_community_pseudonym(signing_key_bytes, governance_key);
    let our_pseudonym_hex = hex::encode(pseudonym.verifying_key().to_bytes());

    // Write join request to inbox
    let inbox_kp_bytes = hex::decode(&metadata.join_inbox_keypair_hex)
        .map_err(|e| TransportError::JoinRejected { community: metadata.name.clone(), reason: format!("invalid inbox keypair: {e}") })?;
    let inbox_kp = crate::broadcast::node::deserialize_keypair(&inbox_kp_bytes)?;
    crate::broadcast::dht_writes::open_writable(node, &metadata.join_inbox_key, inbox_kp).await
        .map_err(|e| TransportError::JoinRejected { community: metadata.name.clone(), reason: format!("cannot open inbox: {e}") })?;
    let subkey_index = super::pseudonym_to_inbox_subkey(&our_pseudonym_hex);
    let mut join_entry = PendingJoinEntry {
        requester_pseudonym_hex: our_pseudonym_hex.clone(), display_name: display_name.to_string(),
        profile_dht_key: session.identity.profile_dht_key.clone(), invite_code_hash: None,
        requested_at: rekindle_utils::timestamp_ms(), status: PendingJoinStatus::Pending,
        signature_hex: String::new(),
    };
    let content = join_entry.signature_content();
    use ed25519_dalek::Signer;
    let signature = pseudonym.sign(&content);
    join_entry.signature_hex = hex::encode(signature.to_bytes());

    // Read-append-write
    let existing = match crate::broadcast::dht_writes::get(node, &metadata.join_inbox_key, subkey_index, true).await {
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
    let entry_bytes = serde_json::to_vec(&entries).map_err(|e| TransportError::SerializationFailed { reason: e.to_string() })?;
    crate::broadcast::dht_writes::set(node, &metadata.join_inbox_key, subkey_index, entry_bytes, None).await
        .map_err(|e| TransportError::JoinRejected { community: metadata.name.clone(), reason: format!("inbox write: {e}") })?;
    info!(community = %metadata.name, subkey = subkey_index, "join request written");

    let registry_key = super::read_registry_key(node, governance_key, &metadata.name).await?;

    Ok(JoinRequestSubmitted {
        community_name: metadata.name,
        governance_key: governance_key.to_string(),
        our_pseudonym_hex,
        registry_key,
        community_mailbox_key: metadata.community_mailbox_key,
    })
}

// ── Phase 2: Await approval ───────────────────────────────────────────

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

    info!(community = community_name, timeout_secs, "awaiting join approval");

    loop {
        if start.elapsed() >= deadline {
            return Err(TransportError::JoinRejected {
                community: community_name.to_string(),
                reason: format!("not approved within {timeout_secs}s — operator may be offline or DHT propagation is slow"),
            });
        }

        tokio::select! {
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
                    Err(_) => { notify_rx = None; }
                }
            }
            () = tokio::time::sleep(poll_interval) => {
                let elapsed = start.elapsed().as_secs();
                let _ = crate::broadcast::dht_writes::open_readonly(node, registry_key).await;
                let members: Vec<MemberSummary> = match crate::broadcast::dht_writes::get(
                    node, registry_key, crate::payload::dht_types::REGISTRY_MEMBER_INDEX, true,
                ).await {
                    Ok(Some(data)) => serde_json::from_slice(&data).unwrap_or_default(),
                    _ => Vec::new(),
                };
                if let Some(m) = members.iter().find(|m| m.pseudonym_key == our_pseudonym_hex) {
                    info!(community = community_name, slot = m.subkey_index, elapsed, "approved via registry poll (tier 3)");
                    return Ok(m.subkey_index);
                }
            }
        }
    }
}

// ── Phase 3: Complete join ────────────────────────────────────────────

pub async fn complete_join(
    node: &TransportNode,
    submitted: &JoinRequestSubmitted,
    slot_index: u32,
    mek_cache: &Arc<RwLock<MekCache>>,
    signing_key_bytes: &[u8; 32],
) -> Result<JoinResult> {
    let dht = node.dht()?;

    let channels = dht.governance().read_channels(&submitted.governance_key).await.unwrap_or_default();
    let channel_summaries: Vec<ChannelEntrySummary> = channels.iter().map(|ch| ChannelEntrySummary {
        id: ch.id.clone(), name: ch.name.clone(), kind: format!("{:?}", ch.kind).to_lowercase(), mek_generation: ch.mek_generation,
    }).collect();

    // Read MEK vault with retry
    let mut meks_cached = 0usize;
    let mut vault = dht.registry().read_mek_vault(&submitted.registry_key).await.unwrap_or_default();

    if !vault.iter().any(|e| e.copies.iter().any(|c| c.target_pseudonym == submitted.our_pseudonym_hex)) {
        info!(community = %submitted.community_name, "MEK vault has no copies for us — retrying with force_refresh");
        let mut backoff = std::time::Duration::from_secs(2);
        let deadline = std::time::Duration::from_secs(20);
        let start = std::time::Instant::now();
        while start.elapsed() < deadline {
            tokio::time::sleep(backoff).await;
            if let Ok(Some(data)) = crate::broadcast::dht_writes::get(
                node, &submitted.registry_key,
                crate::payload::dht_types::REGISTRY_MEK_VAULT, true,
            ).await {
                let fresh: Vec<crate::payload::dht_types::MekVaultEntry> = serde_json::from_slice(&data).unwrap_or_default();
                if fresh.iter().any(|e| e.copies.iter().any(|c| c.target_pseudonym == submitted.our_pseudonym_hex)) {
                    vault = fresh;
                    info!(community = %submitted.community_name, elapsed_ms = start.elapsed().as_millis(), "MEK vault propagated");
                    break;
                }
            }
            backoff = (backoff * 2).min(std::time::Duration::from_secs(5));
        }
    }

    for entry in &vault {
        if let Some(copy) = entry.copies.iter().find(|c| c.target_pseudonym == submitted.our_pseudonym_hex) {
            let transfer = crate::payload::rpc::MekTransferPayload {
                channel_id: entry.channel_id.clone(), generation: entry.generation,
                rotator_pseudonym_hex: entry.rotator_pseudonym.clone(), wrapped_mek: copy.encrypted_mek.clone(),
            };
            match crate::operations::mek::receive_mek_transfer_payload(&transfer, signing_key_bytes, &submitted.governance_key, mek_cache) {
                Ok(_) => { meks_cached += 1; }
                Err(e) => { tracing::warn!(channel = %entry.channel_id, error = %e, "MEK vault unwrap failed"); }
            }
        }
    }
    info!(community = %submitted.community_name, meks_cached, "MEKs cached");

    let slot_seed = super::derive_slot_seed(signing_key_bytes, &submitted.governance_key, slot_index);
    Ok(JoinResult {
        community_name: submitted.community_name.clone(),
        governance_key: submitted.governance_key.clone(),
        our_pseudonym_key: submitted.our_pseudonym_hex.clone(),
        display_name: String::new(),
        our_slot_index: slot_index,
        registry_key: submitted.registry_key.clone(),
        community_mailbox_key: submitted.community_mailbox_key.clone(),
        channels: channel_summaries, meks_cached, slot_seed,
    })
}

// ── Convenience wrapper ───────────────────────────────────────────────

pub async fn join_community(
    node: &TransportNode, session: &Session, governance_key: &str,
    display_name: &str, mek_cache: &Arc<RwLock<MekCache>>, signing_key_bytes: &[u8; 32],
) -> Result<JoinResult> {
    let submitted = submit_join_request(node, session, governance_key, display_name, signing_key_bytes).await?;
    let slot_index = await_join_approval(node, &submitted.registry_key, &submitted.our_pseudonym_hex, &submitted.community_name, None, 120).await?;
    let mut result = complete_join(node, &submitted, slot_index, mek_cache, signing_key_bytes).await?;
    result.display_name = display_name.to_string();
    Ok(result)
}
