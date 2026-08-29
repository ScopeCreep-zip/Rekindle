//! Inbox processing stages: leave pruning, join registration, and
//! post-join notification.

use parking_lot::RwLock;

use rekindle_transport::payload::dht_types::MemberSummary;
use rekindle_utils::timestamp_ms as now_ms;

use super::get_signing_key;

/// Process `Left` inbox entries: drop the members, prune their vault copies,
/// then rekey every channel for forward secrecy.
pub(super) async fn process_inbox_leaves(
    dht: &rekindle_transport::DhtStore,
    registry_key: &str,
    governance_key: &str,
    members: &mut Vec<MemberSummary>,
    channels: &[rekindle_transport::payload::dht_types::ChannelEntry],
    signing_key: &RwLock<Option<crate::state::keystore::SigningKeyHandle>>,
    mek_cache: &RwLock<rekindle_transport::crypto::mek::MekCache>,
    pending: &[rekindle_transport::payload::dht_types::PendingJoinEntry],
) {
    let mut left_members = Vec::new();
    for req in pending {
        if matches!(
            req.status,
            rekindle_transport::payload::dht_types::PendingJoinStatus::Left { .. }
        ) {
            left_members.push(req.requester_pseudonym_hex.clone());
        }
    }
    if left_members.is_empty() {
        return;
    }
    let before = members.len();
    for pseudonym in &left_members {
        members.retain(|m| m.pseudonym_key != *pseudonym);
    }
    if members.len() >= before {
        return;
    }
    let _ = dht
        .registry()
        .write_member_index(registry_key, members)
        .await;
    tracing::info!(
        removed = before - members.len(),
        "inbox: processed leave entries"
    );

    // Remove vault copies for left members
    let mut vault = dht
        .registry()
        .read_mek_vault(registry_key)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "DHT read failed, using empty");
            Vec::new()
        });
    for entry in &mut vault {
        entry
            .copies
            .retain(|c| !left_members.contains(&c.target_pseudonym));
    }
    let _ = dht.registry().write_mek_vault(registry_key, &vault).await;

    // Rekey for forward secrecy
    let Some(sk) = get_signing_key(signing_key) else {
        return;
    };
    let ps = rekindle_transport::crypto::pseudonym::derive_community_pseudonym(&sk, governance_key);
    let ps_hex = hex::encode(ps.verifying_key().to_bytes());
    let mut new_vault = Vec::new();
    for channel in channels {
        let gen = mek_cache
            .read()
            .current(governance_key, &channel.id)
            .map_or(0, rekindle_transport::crypto::mek::Mek::generation)
            + 1;
        let new_mek = rekindle_transport::crypto::mek::Mek::generate(gen);
        let mek_wire = new_mek.to_wire_bytes();
        let copies = members
            .iter()
            .filter_map(|m| {
                let pub_bytes: [u8; 32] = hex::decode(&m.pseudonym_key).ok()?.try_into().ok()?;
                rekindle_transport::crypto::mek::wrap_mek(&ps, &pub_bytes, &mek_wire)
                    .ok()
                    .map(
                        |wrapped| rekindle_transport::payload::dht_types::EncryptedMekCopy {
                            target_pseudonym: m.pseudonym_key.clone(),
                            encrypted_mek: wrapped,
                        },
                    )
            })
            .collect();
        new_vault.push(rekindle_transport::payload::dht_types::MekVaultEntry {
            channel_id: channel.id.clone(),
            generation: gen,
            rotator_pseudonym: ps_hex.clone(),
            copies,
        });
        mek_cache
            .write()
            .insert(governance_key, &channel.id, new_mek);
    }
    if !new_vault.is_empty() {
        let _ = dht
            .registry()
            .write_mek_vault(registry_key, &new_vault)
            .await;
    }
    tracing::info!(rekeyed = new_vault.len(), "inbox: rekeyed after leave");
}

/// Process pending join requests: verify signatures, apply join policy, and
/// register accepted members. Returns the count of newly registered members.
pub(super) async fn process_inbox_joins(
    dht: &rekindle_transport::DhtStore,
    registry_key: &str,
    metadata: &rekindle_transport::payload::dht_types::CommunityMetadata,
    bans: &[rekindle_transport::payload::dht_types::BanEntry],
    members: &mut Vec<MemberSummary>,
    pending: &[rekindle_transport::payload::dht_types::PendingJoinEntry],
) -> u32 {
    let mut new_members = 0u32;
    for req in pending {
        // Skip leave entries (already processed above)
        if matches!(
            req.status,
            rekindle_transport::payload::dht_types::PendingJoinStatus::Left { .. }
        ) {
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
                let pub_bytes: [u8; 32] = hex::decode(&req.requester_pseudonym_hex)
                    .ok()?
                    .try_into()
                    .ok()?;
                let verifying_key = ed25519_dalek::VerifyingKey::from_bytes(&pub_bytes).ok()?;
                let signature = ed25519_dalek::Signature::from_bytes(&sig_bytes);
                let content = req.signature_content();
                Some(verifying_key.verify_strict(&content, &signature).is_ok())
            })()
            .unwrap_or(false);
            if !sig_ok {
                tracing::warn!(
                    requester = %&req.requester_pseudonym_hex[..16.min(req.requester_pseudonym_hex.len())],
                    "inbox: SIGNATURE VERIFICATION FAILED — forged entry, skipping"
                );
                continue;
            }
        }
        // Skip banned
        if bans
            .iter()
            .any(|b| b.pseudonym_key == req.requester_pseudonym_hex)
        {
            tracing::info!(requester = %&req.requester_pseudonym_hex[..16.min(req.requester_pseudonym_hex.len())], "inbox: banned, skipping");
            continue;
        }
        // Skip already member
        if members
            .iter()
            .any(|m| m.pseudonym_key == req.requester_pseudonym_hex)
        {
            tracing::debug!(requester = %&req.requester_pseudonym_hex[..16.min(req.requester_pseudonym_hex.len())], "inbox: already member, skipping");
            continue;
        }
        // Check join policy
        match metadata.join_policy {
            rekindle_transport::payload::dht_types::JoinPolicy::AutoAllow => {}
            rekindle_transport::payload::dht_types::JoinPolicy::WaitingRoom => {
                // Add to moderation queue for manual approval
                let mut queue = dht
                    .registry()
                    .read_moderation_queue(registry_key)
                    .await
                    .unwrap_or_else(|e| {
                        tracing::warn!(error = %e, "DHT read failed, using empty");
                        Vec::new()
                    });
                if !queue
                    .iter()
                    .any(|p| p.requester_pseudonym_hex == req.requester_pseudonym_hex)
                {
                    queue.push(req.clone());
                    let _ = dht
                        .registry()
                        .write_moderation_queue(registry_key, &queue)
                        .await;
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
        let slot = members
            .iter()
            .map(|m| m.subkey_index)
            .max()
            .map_or(1, |m| m + 1)
            .max(1);
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
    new_members
}

/// Send tier-2 `JoinAccepted` notifications directly to each newly approved
/// member's route, carrying their wrapped MEK for instant decryption.
pub(super) async fn notify_new_members(
    transport_node: &rekindle_transport::TransportNode,
    governance_key: &str,
    registry_key: &str,
    members: &[MemberSummary],
    vault: &[rekindle_transport::payload::dht_types::MekVaultEntry],
    signing_key_bytes: &[u8; 32],
    bans: &[rekindle_transport::payload::dht_types::BanEntry],
    pending: &[rekindle_transport::payload::dht_types::PendingJoinEntry],
) {
    for req in pending {
        if bans
            .iter()
            .any(|b| b.pseudonym_key == req.requester_pseudonym_hex)
        {
            continue;
        }
        if matches!(
            req.status,
            rekindle_transport::payload::dht_types::PendingJoinStatus::Left { .. }
        ) {
            continue;
        }
        if req.profile_dht_key.is_empty() {
            continue;
        }

        // Find the slot we assigned to this member
        let Some(member) = members
            .iter()
            .find(|m| m.pseudonym_key == req.requester_pseudonym_hex)
        else {
            continue;
        };
        let slot = member.subkey_index;

        // Read joiner's route blob from their profile DHT record
        let _ = rekindle_transport::broadcast::dht_writes::open_readonly(
            transport_node,
            &req.profile_dht_key,
        )
        .await;
        let route_blob = match rekindle_transport::broadcast::dht_writes::get(
            transport_node,
            &req.profile_dht_key,
            rekindle_transport::payload::dht_types::PROFILE_SUBKEY_ROUTE_BLOB,
            false,
        )
        .await
        {
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
        let (mek_for_joiner, mek_gen) = vault
            .iter()
            .find_map(|entry| {
                entry
                    .copies
                    .iter()
                    .find(|c| c.target_pseudonym == req.requester_pseudonym_hex)
                    .map(|c| (c.encrypted_mek.clone(), entry.generation))
            })
            .unwrap_or((Vec::new(), 0));

        // Send JoinAccepted directly to joiner via transport's send_direct
        let join_accepted = rekindle_transport::payload::gossip::GossipPayload::Control(
            rekindle_transport::payload::gossip::ControlPayload::JoinAccepted {
                mek_encrypted: mek_for_joiner,
                mek_generation: mek_gen,
                member_registry_key: Some(registry_key.to_string()),
                slot_index: Some(slot),
                wrapped_slot_seed: None,
            },
        );
        let report = rekindle_transport::broadcast::gossip::send_direct(
            transport_node,
            governance_key,
            &members[0].pseudonym_key,
            signing_key_bytes,
            join_accepted,
            &req.requester_pseudonym_hex,
            &route_blob,
        )
        .await;
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
}
