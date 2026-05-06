//! Community RPC handlers and DHT inbox processor.
//!
//! Join is fully DHT-based — the owner's daemon polls the join inbox
//! record and processes pending requests by writing to the registry.
//!
//! Leave notification is still best-effort RPC (fire-and-forget from
//! the leaving member to the community route for cleanup + rekey).


use std::sync::Arc;
use std::time::Duration;

use parking_lot::RwLock;

use rekindle_transport::payload::rpc::{
    CallResponse, CommunityLeaveNotification,
};
use rekindle_transport::payload::dht_types::MemberSummary;

/// Maximum time the leave handler may run before returning.
pub(crate) const HANDLER_DEADLINE: Duration = Duration::from_secs(12);

/// Module-level cache for registry keypairs loaded from the OS keyring.
static REGISTRY_KEYPAIR_CACHE: std::sync::LazyLock<parking_lot::Mutex<std::collections::HashMap<String, Vec<u8>>>> =
    std::sync::LazyLock::new(|| parking_lot::Mutex::new(std::collections::HashMap::new()));

#[allow(clippy::cast_possible_truncation)]
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ── DHT Inbox Processor ───────────────────────────────────────────────

/// Process pending join requests from the inbox DHT record.
///
/// Called periodically by the owner's daemon for each operator community.
/// Uses inspect-first optimization to read only populated subkeys.
///
/// SECURITY NOTE (v1 limitation): The join inbox uses a DFLT record with a
/// published keypair. Any node can write to any subkey. Each entry's
/// `signature_hex` is verified against `requester_pseudonym_hex` before
/// approval to prevent impersonation. Entries without signatures are
/// processed with a warning (legacy migration path). A SMPL-based inbox
/// with per-joiner writer slots is planned for v2.
pub async fn process_inbox(
    session: &RwLock<Option<rekindle_transport::Session>>,
    signing_key: &RwLock<Option<crate::state::keystore::SigningKeyHandle>>,
    mek_cache: &RwLock<rekindle_transport::crypto::mek::MekCache>,
    transport: &RwLock<Option<Arc<rekindle_transport::TransportNode>>>,
    session_path: &std::path::Path,
    governance_key: &str,
) {
    let Some(registry_key) = require_operator_registry(session, governance_key) else {
        tracing::trace!(governance_key, "inbox: not operator for this community");
        return;
    };
    let Some(signing_key_bytes) = get_signing_key(signing_key) else {
        tracing::warn!(governance_key, "inbox: signing key not available (daemon locked?)");
        return;
    };
    let Some(transport_node) = get_transport(transport) else {
        tracing::warn!(governance_key, "inbox: transport not started");
        return;
    };
    let Ok(dht) = transport_node.dht() else {
        tracing::warn!(governance_key, "inbox: DHT access failed");
        return;
    };

    // Read metadata for inbox key
    if let Err(e) = rekindle_transport::broadcast::dht_writes::open_readonly(&transport_node, governance_key).await {
        tracing::warn!(governance_key, error = %e, "inbox: cannot open governance record");
        return;
    }
    let metadata = match dht.governance().read_metadata(governance_key).await {
        Ok(Some(m)) => m,
        Ok(None) => {
            tracing::warn!(governance_key, "inbox: governance metadata is None");
            return;
        }
        Err(e) => {
            tracing::warn!(governance_key, error = %e, "inbox: governance metadata read failed");
            return;
        }
    };
    // If join_inbox_key is empty, the local cache may be stale (initial metadata
    // from step 1 of create, before step 9 wrote the final metadata). Retry with
    // force_refresh=true to pull the latest from the DHT network.
    let metadata = if metadata.join_inbox_key.is_empty() {
        if let Ok(Some(data)) = rekindle_transport::broadcast::dht_writes::get(
            &transport_node, governance_key,
            rekindle_transport::payload::dht_types::MANIFEST_METADATA, true,
        ).await {
            match serde_json::from_slice::<rekindle_transport::payload::dht_types::CommunityMetadata>(&data) {
                Ok(m) if !m.join_inbox_key.is_empty() => m,
                Ok(_) => {
                    tracing::warn!(governance_key, "inbox: join_inbox_key still empty after network refresh");
                    return;
                }
                Err(e) => {
                    tracing::warn!(governance_key, error = %e, "inbox: metadata parse failed after refresh");
                    return;
                }
            }
        } else {
            tracing::warn!(governance_key, "inbox: metadata fetch failed on network refresh");
            return;
        }
    } else {
        metadata
    };

    // Open inbox readonly to read pending requests
    if let Err(e) = rekindle_transport::broadcast::dht_writes::open_readonly(&transport_node, &metadata.join_inbox_key).await {
        tracing::warn!(inbox_key = %metadata.join_inbox_key, error = %e, "inbox: cannot open inbox record");
        return;
    }
    let pending = match rekindle_transport::operations::community::read_inbox_requests(&dht, &metadata.join_inbox_key).await {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(inbox_key = %metadata.join_inbox_key, error = %e, "inbox: read_inbox_requests failed");
            return;
        }
    };

    if pending.is_empty() {
        tracing::trace!(governance_key, "inbox: no pending requests");
        return;
    }
    tracing::info!(community = %metadata.name, requests = pending.len(), "processing join inbox");

    // Open registry writable
    open_registry_writable(&transport_node, &registry_key).await;

    // Read current state
    let bans = dht.governance().read_bans(governance_key).await.unwrap_or_else(|e| { tracing::warn!(error = %e, "DHT read failed, using empty"); Vec::new() });
    let mut members = dht.registry().read_member_index(&registry_key).await.unwrap_or_else(|e| { tracing::warn!(error = %e, "DHT read failed, using empty"); Vec::new() });
    let channels = dht.governance().read_channels(governance_key).await.unwrap_or_else(|e| { tracing::warn!(error = %e, "DHT read failed, using empty"); Vec::new() });

    // Process leave entries first — remove members and rekey
    let mut left_members = Vec::new();
    for req in &pending {
        if matches!(req.status, rekindle_transport::payload::dht_types::PendingJoinStatus::Left { .. }) {
            left_members.push(req.requester_pseudonym_hex.clone());
        }
    }
    if !left_members.is_empty() {
        let before = members.len();
        for pseudonym in &left_members {
            members.retain(|m| m.pseudonym_key != *pseudonym);
        }
        if members.len() < before {
            let _ = dht.registry().write_member_index(&registry_key, &members).await;
            tracing::info!(removed = before - members.len(), "inbox: processed leave entries");

            // Remove vault copies for left members
            let mut vault = dht.registry().read_mek_vault(&registry_key).await.unwrap_or_else(|e| { tracing::warn!(error = %e, "DHT read failed, using empty"); Vec::new() });
            for entry in &mut vault {
                entry.copies.retain(|c| !left_members.contains(&c.target_pseudonym));
            }
            let _ = dht.registry().write_mek_vault(&registry_key, &vault).await;

            // Rekey for forward secrecy
            if let Some(sk) = get_signing_key(signing_key) {
                let ps = rekindle_transport::crypto::pseudonym::derive_community_pseudonym(&sk, governance_key);
                let ps_hex = hex::encode(ps.verifying_key().to_bytes());
                let mut new_vault = Vec::new();
                for channel in &channels {
                    let gen = mek_cache.read()
                        .current(governance_key, &channel.id)
                        .map_or(0, rekindle_transport::crypto::mek::Mek::generation) + 1;
                    let new_mek = rekindle_transport::crypto::mek::Mek::generate(gen);
                    let mek_wire = new_mek.to_wire_bytes();
                    let copies = members.iter().filter_map(|m| {
                        let pub_bytes: [u8; 32] = hex::decode(&m.pseudonym_key).ok()?.try_into().ok()?;
                        rekindle_transport::crypto::mek::wrap_mek(&ps, &pub_bytes, &mek_wire).ok().map(|wrapped| {
                            rekindle_transport::payload::dht_types::EncryptedMekCopy {
                                target_pseudonym: m.pseudonym_key.clone(), encrypted_mek: wrapped,
                            }
                        })
                    }).collect();
                    new_vault.push(rekindle_transport::payload::dht_types::MekVaultEntry {
                        channel_id: channel.id.clone(), generation: gen,
                        rotator_pseudonym: ps_hex.clone(), copies,
                    });
                    mek_cache.write().insert(governance_key, &channel.id, new_mek);
                }
                if !new_vault.is_empty() {
                    let _ = dht.registry().write_mek_vault(&registry_key, &new_vault).await;
                }
                tracing::info!(rekeyed = new_vault.len(), "inbox: rekeyed after leave");
            }
        }
    }

    // Process join entries
    let mut new_members = 0u32;
    for req in &pending {
        // Skip leave entries (already processed above)
        if matches!(req.status, rekindle_transport::payload::dht_types::PendingJoinStatus::Left { .. }) {
            continue;
        }
        // Verify Ed25519 signature to prevent impersonation via shared inbox keypair
        if req.signature_hex.is_empty() {
            tracing::warn!(
                requester = %&req.display_name,
                "inbox: entry has no signature — processing with caution"
            );
        } else {
            let sig_ok = (|| -> Option<bool> {
                let sig_bytes: [u8; 64] = hex::decode(&req.signature_hex).ok()?.try_into().ok()?;
                let pub_bytes: [u8; 32] = hex::decode(&req.requester_pseudonym_hex).ok()?.try_into().ok()?;
                let verifying_key = ed25519_dalek::VerifyingKey::from_bytes(&pub_bytes).ok()?;
                let signature = ed25519_dalek::Signature::from_bytes(&sig_bytes);
                let content = req.signature_content();
                use ed25519_dalek::Verifier;
                Some(verifying_key.verify(&content, &signature).is_ok())
            })().unwrap_or(false);
            if !sig_ok {
                tracing::warn!(
                    requester = %&req.requester_pseudonym_hex[..16.min(req.requester_pseudonym_hex.len())],
                    "inbox: SIGNATURE VERIFICATION FAILED — forged entry, skipping"
                );
                continue;
            }
        }
        // Skip banned
        if bans.iter().any(|b| b.pseudonym_key == req.requester_pseudonym_hex) {
            tracing::info!(requester = %&req.requester_pseudonym_hex[..16.min(req.requester_pseudonym_hex.len())], "inbox: banned, skipping");
            continue;
        }
        // Skip already member
        if members.iter().any(|m| m.pseudonym_key == req.requester_pseudonym_hex) {
            tracing::debug!(requester = %&req.requester_pseudonym_hex[..16.min(req.requester_pseudonym_hex.len())], "inbox: already member, skipping");
            continue;
        }
        // Check join policy
        match metadata.join_policy {
            rekindle_transport::payload::dht_types::JoinPolicy::AutoAllow => {}
            rekindle_transport::payload::dht_types::JoinPolicy::WaitingRoom => {
                // Add to moderation queue for manual approval
                let mut queue = dht.registry().read_moderation_queue(&registry_key).await.unwrap_or_else(|e| { tracing::warn!(error = %e, "DHT read failed, using empty"); Vec::new() });
                if !queue.iter().any(|p| p.requester_pseudonym_hex == req.requester_pseudonym_hex) {
                    queue.push(req.clone());
                    let _ = dht.registry().write_moderation_queue(&registry_key, &queue).await;
                    tracing::info!(requester = %&req.display_name, "inbox: added to waiting room");
                }
                continue;
            }
            rekindle_transport::payload::dht_types::JoinPolicy::InviteOnly => {
                if req.invite_code_hash.is_none() {
                    tracing::info!(requester = %&req.display_name, "inbox: no invite code, skipping");
                    continue;
                }
                // TODO: validate invite code against governance invites
            }
        }

        // Register member
        let slot = members.iter().map(|m| m.subkey_index).max().map_or(1, |m| m + 1).max(1);
        members.push(MemberSummary {
            pseudonym_key: req.requester_pseudonym_hex.clone(),
            display_name: req.display_name.clone(),
            role_ids: Vec::new(),
            joined_at: now_ms(),
            subkey_index: slot,
            onboarding_complete: true,
            timeout_until: None,
            profile_dht_key: Some(req.profile_dht_key.clone()),
            channel_records: std::collections::HashMap::new(),
        });
        new_members += 1;
        tracing::info!(member = %req.display_name, slot, "inbox: member registered");
    }

    if new_members == 0 { return }

    // Write updated member index
    if let Err(e) = dht.registry().write_member_index(&registry_key, &members).await {
        tracing::error!(error = %e, "inbox: failed to write member index");
        return;
    }

    // Wrap MEKs for all new members and update vault
    let mut vault = dht.registry().read_mek_vault(&registry_key).await.unwrap_or_else(|e| { tracing::warn!(error = %e, "DHT read failed, using empty"); Vec::new() });
    for req in &pending {
        if bans.iter().any(|b| b.pseudonym_key == req.requester_pseudonym_hex) { continue }
        if matches!(metadata.join_policy, rekindle_transport::payload::dht_types::JoinPolicy::WaitingRoom) { continue }

        match rekindle_transport::operations::mek::wrap_meks_for_member(
            &channels, &req.requester_pseudonym_hex, &signing_key_bytes, governance_key, mek_cache,
        ) {
            Ok(transfers) => {
                for t in &transfers {
                    if let Some(entry) = vault.iter_mut().find(|e| e.channel_id == t.channel_id) {
                        entry.copies.push(rekindle_transport::payload::dht_types::EncryptedMekCopy {
                            target_pseudonym: req.requester_pseudonym_hex.clone(),
                            encrypted_mek: t.wrapped_mek.clone(),
                        });
                    }
                }
            }
            Err(e) => tracing::warn!(error = %e, "inbox: MEK wrap failed"),
        }
    }
    let _ = dht.registry().write_mek_vault(&registry_key, &vault).await;

    // Direct notification to newly approved members (tier 2 — instant).
    // Sends a signed GossipPayload::Control(JoinAccepted) directly to the
    // joiner's route. The joiner's on_gossip handler sees it and completes
    // the pending_join oneshot, unblocking the join immediately.
    for req in &pending {
        if bans.iter().any(|b| b.pseudonym_key == req.requester_pseudonym_hex) { continue; }
        if matches!(req.status, rekindle_transport::payload::dht_types::PendingJoinStatus::Left { .. }) { continue; }
        if req.profile_dht_key.is_empty() { continue; }

        // Find the slot we assigned to this member
        let Some(member) = members.iter().find(|m| m.pseudonym_key == req.requester_pseudonym_hex) else {
            continue;
        };
        let slot = member.subkey_index;

        // Read joiner's route blob from their profile DHT record
        let _ = rekindle_transport::broadcast::dht_writes::open_readonly(&transport_node, &req.profile_dht_key).await;
        let route_blob = match rekindle_transport::broadcast::dht_writes::get(
            &transport_node, &req.profile_dht_key,
            rekindle_transport::payload::dht_types::PROFILE_SUBKEY_ROUTE_BLOB, false,
        ).await {
            Ok(Some(blob)) if !blob.is_empty() => {
                tracing::info!(member = %req.display_name, blob_bytes = blob.len(), "inbox: joiner route blob read from profile");
                blob
            }
            Ok(Some(_) | None) => {
                tracing::warn!(member = %req.display_name, profile = %req.profile_dht_key, "inbox: joiner route blob empty or missing — cannot send direct notification");
                continue;
            }
            Err(e) => {
                tracing::warn!(member = %req.display_name, error = %e, "inbox: joiner profile read failed — cannot send direct notification");
                continue;
            }
        };

        // Look up the wrapped MEK for this specific joiner from the vault we just updated.
        // Carrying it in the notification means the joiner has the MEK instantly — no
        // DHT vault read needed, no propagation dependency.
        let (mek_for_joiner, mek_gen) = vault.iter()
            .find_map(|entry| {
                entry.copies.iter()
                    .find(|c| c.target_pseudonym == req.requester_pseudonym_hex)
                    .map(|c| (c.encrypted_mek.clone(), entry.generation))
            })
            .unwrap_or((Vec::new(), 0));

        // Send JoinAccepted directly to joiner via transport's send_direct
        let join_accepted = rekindle_transport::payload::gossip::GossipPayload::Control(
            rekindle_transport::payload::gossip::ControlPayload::JoinAccepted {
                mek_encrypted: mek_for_joiner,
                mek_generation: mek_gen,
                member_registry_key: Some(registry_key.clone()),
                slot_index: Some(slot),
                wrapped_slot_seed: None,
            },
        );
        let report = rekindle_transport::broadcast::gossip::send_direct(
            &transport_node, governance_key, &members[0].pseudonym_key,
            &signing_key_bytes, join_accepted,
            &req.requester_pseudonym_hex, &route_blob,
        ).await;
        if report.delivered > 0 {
            tracing::info!(member = %req.display_name, slot, "inbox: JoinAccepted sent directly to joiner (tier 2)");
        } else {
            tracing::warn!(
                member = %req.display_name, slot,
                failures = ?report.failures,
                "inbox: JoinAccepted direct send FAILED — joiner must rely on registry poll (tier 3)"
            );
        }
    }

    // Save session
    { let guard = session.read(); if let Some(ref s) = *guard { let _ = s.save(session_path); } }
    tracing::info!(community = %metadata.name, new_members, total = members.len(), "inbox processing complete");
}

// ── Leave handler (still RPC-based, best-effort) ──────────────────────

/// Leave handler — remove member, rekey for forward secrecy.
pub(crate) async fn handle_leave(
    notif: &CommunityLeaveNotification,
    session: &RwLock<Option<rekindle_transport::Session>>,
    signing_key: &RwLock<Option<crate::state::keystore::SigningKeyHandle>>,
    mek_cache: &RwLock<rekindle_transport::crypto::mek::MekCache>,
    transport: &RwLock<Option<Arc<rekindle_transport::TransportNode>>>,
    session_path: &std::path::Path,
) -> CallResponse {
    if let Ok(response) = tokio::time::timeout(HANDLER_DEADLINE, handle_leave_inner(
        notif, session, signing_key, mek_cache, transport, session_path,
    )).await {
        response
    } else {
        tracing::error!(
            member = %&notif.leaving_pseudonym_hex[..16.min(notif.leaving_pseudonym_hex.len())],
            "leave handler exceeded deadline"
        );
        CallResponse::Ack
    }
}

async fn handle_leave_inner(
    notif: &CommunityLeaveNotification,
    session: &RwLock<Option<rekindle_transport::Session>>,
    signing_key: &RwLock<Option<crate::state::keystore::SigningKeyHandle>>,
    mek_cache: &RwLock<rekindle_transport::crypto::mek::MekCache>,
    transport: &RwLock<Option<Arc<rekindle_transport::TransportNode>>>,
    session_path: &std::path::Path,
) -> CallResponse {
    tracing::info!(
        community = %&notif.governance_key[..16.min(notif.governance_key.len())],
        member = %&notif.leaving_pseudonym_hex[..16.min(notif.leaving_pseudonym_hex.len())],
        "processing leave"
    );

    let Some(registry_key) = require_operator_registry(session, &notif.governance_key) else {
        return CallResponse::Ack;
    };
    let Some(transport_node) = get_transport(transport) else { return CallResponse::Ack };
    let Ok(dht) = transport_node.dht() else { return CallResponse::Ack };

    open_registry_writable(&transport_node, &registry_key).await;

    // Remove from index
    let mut members = dht.registry().read_member_index(&registry_key).await.unwrap_or_else(|e| { tracing::warn!(error = %e, "DHT read failed, using empty"); Vec::new() });
    let before = members.len();
    members.retain(|m| m.pseudonym_key != notif.leaving_pseudonym_hex);
    if members.len() < before {
        let _ = dht.registry().write_member_index(&registry_key, &members).await;
        tracing::info!(remaining = members.len(), "member removed");
    }

    // Remove vault copies
    let mut vault = dht.registry().read_mek_vault(&registry_key).await.unwrap_or_else(|e| { tracing::warn!(error = %e, "DHT read failed, using empty"); Vec::new() });
    for entry in &mut vault {
        entry.copies.retain(|c| c.target_pseudonym != notif.leaving_pseudonym_hex);
    }
    let _ = dht.registry().write_mek_vault(&registry_key, &vault).await;

    // Rekey for forward secrecy
    let Some(signing_key_bytes) = get_signing_key(signing_key) else {
        tracing::warn!("locked — cannot rekey");
        return CallResponse::Ack;
    };
    let channels = dht.governance().read_channels(&notif.governance_key).await.unwrap_or_else(|e| { tracing::warn!(error = %e, "DHT read failed, using empty"); Vec::new() });
    let our_ps = rekindle_transport::crypto::pseudonym::derive_community_pseudonym(
        &signing_key_bytes, &notif.governance_key,
    );
    let our_ps_hex = hex::encode(our_ps.verifying_key().to_bytes());

    let mut new_vault = Vec::new();
    for channel in &channels {
        let gen = mek_cache.read()
            .current(&notif.governance_key, &channel.id)
            .map_or(0, rekindle_transport::crypto::mek::Mek::generation) + 1;
        let new_mek = rekindle_transport::crypto::mek::Mek::generate(gen);
        let mek_wire = new_mek.to_wire_bytes();
        let copies = members.iter().filter_map(|m| {
            let pub_bytes: [u8; 32] = hex::decode(&m.pseudonym_key).ok()?.try_into().ok()?;
            rekindle_transport::crypto::mek::wrap_mek(&our_ps, &pub_bytes, &mek_wire).ok().map(|wrapped| {
                rekindle_transport::payload::dht_types::EncryptedMekCopy {
                    target_pseudonym: m.pseudonym_key.clone(), encrypted_mek: wrapped,
                }
            })
        }).collect();
        new_vault.push(rekindle_transport::payload::dht_types::MekVaultEntry {
            channel_id: channel.id.clone(), generation: gen,
            rotator_pseudonym: our_ps_hex.clone(), copies,
        });
        mek_cache.write().insert(&notif.governance_key, &channel.id, new_mek);
    }
    if !new_vault.is_empty() {
        let _ = dht.registry().write_mek_vault(&registry_key, &new_vault).await;
    }

    { let guard = session.read(); if let Some(ref s) = *guard { let _ = s.save(session_path); } }
    tracing::info!(rekeyed = new_vault.len(), remaining = members.len(), "leave + rekey complete");
    CallResponse::Ack
}

// ── Helpers ─────────────────────────────────────────────────────────────

/// Load registry keypair bytes, using module-level cache.
pub(crate) async fn load_registry_keypair(registry_key: &str) -> Option<Vec<u8>> {
    let short = if registry_key.len() > 12 { &registry_key[..12] } else { registry_key };
    let label = format!("registry-{short}");
    {
        let cache = REGISTRY_KEYPAIR_CACHE.lock();
        if let Some(bytes) = cache.get(&label) {
            return Some(bytes.clone());
        }
    }
    match crate::state::keystore::load_keypair_bytes(&label).await {
        Ok(Some(bytes)) => {
            REGISTRY_KEYPAIR_CACHE.lock().insert(label, bytes.clone());
            Some(bytes)
        }
        _ => None,
    }
}

/// Open a registry record writable using the cached keypair.
pub(crate) async fn open_registry_writable(
    node: &rekindle_transport::TransportNode,
    registry_key: &str,
) -> bool {
    let Some(kp_bytes) = load_registry_keypair(registry_key).await else {
        tracing::warn!("registry keypair not in keyring — opening readonly");
        let _ = rekindle_transport::broadcast::dht_writes::open_readonly(node, registry_key).await;
        return false;
    };
    let Ok(kp) = rekindle_transport::deserialize_keypair(&kp_bytes) else {
        tracing::warn!("registry keypair deserialize failed — opening readonly");
        let _ = rekindle_transport::broadcast::dht_writes::open_readonly(node, registry_key).await;
        return false;
    };
    match rekindle_transport::broadcast::dht_writes::open_writable(node, registry_key, kp).await {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!(error = %e, "registry open writable failed — falling back to readonly");
            let _ = rekindle_transport::broadcast::dht_writes::open_readonly(node, registry_key).await;
            false
        }
    }
}

/// Validate operator status for a community, return its registry key.
pub(crate) fn require_operator_registry(
    session: &RwLock<Option<rekindle_transport::Session>>,
    governance_key: &str,
) -> Option<String> {
    let guard = session.read();
    let sess = guard.as_ref()?;
    let membership = sess.community(governance_key)?;
    if !membership.is_operator { return None; }
    Some(membership.registry_key.clone())
}

pub(crate) fn get_signing_key(
    signing_key: &RwLock<Option<crate::state::keystore::SigningKeyHandle>>,
) -> Option<[u8; 32]> {
    signing_key.read().as_ref().map(|h| *h.as_bytes())
}

pub(crate) fn get_transport(
    transport: &RwLock<Option<Arc<rekindle_transport::TransportNode>>>,
) -> Option<Arc<rekindle_transport::TransportNode>> {
    transport.read().as_ref().map(Arc::clone)
}
