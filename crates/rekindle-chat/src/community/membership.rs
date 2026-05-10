//! Member registry operations — list, add, remove, approve, reject,
//! pending queue, ownership transfer, join inbox processing, leave handler.

use std::collections::HashMap;

use rekindle_types::dht_types::{
    CommunityMetadata, EncryptedMekCopy, MemberSummary, MekVaultEntry,
    PendingJoinEntry, PendingJoinStatus,
    MANIFEST_METADATA, REGISTRY_MEMBER_INDEX, REGISTRY_MEK_VAULT,
    REGISTRY_MODERATION_QUEUE,
};

use crate::time::timestamp_ms;
use crate::ChatError;
use super::CommunityService;

impl CommunityService {
    // ── Internal helpers ─────────────────────────────────────────

    pub(crate) async fn read_members(
        &self, registry_key: &str,
    ) -> Result<Vec<MemberSummary>, ChatError> {
        let data = self.io
            .read_record(registry_key, REGISTRY_MEMBER_INDEX, false).await?;
        match data {
            Some(bytes) if !bytes.is_empty() => serde_json::from_slice(&bytes)
                .map_err(|e| ChatError::Deserialization(format!("members: {e}"))),
            _ => Ok(Vec::new()),
        }
    }

    pub(crate) async fn read_moderation_queue(
        &self, registry_key: &str,
    ) -> Result<Vec<PendingJoinEntry>, ChatError> {
        let data = self.io
            .read_record(registry_key, REGISTRY_MODERATION_QUEUE, false).await?;
        match data {
            Some(bytes) if !bytes.is_empty() => serde_json::from_slice(&bytes)
                .map_err(|e| ChatError::Deserialization(format!("queue: {e}"))),
            _ => Ok(Vec::new()),
        }
    }

    pub(crate) async fn write_moderation_queue(
        &self, registry_key: &str, queue: &[PendingJoinEntry], keypair: &[u8],
    ) -> Result<(), ChatError> {
        let bytes = serde_json::to_vec(queue)
            .map_err(|e| ChatError::Serialization(format!("queue: {e}")))?;
        self.io
            .write_record(registry_key, REGISTRY_MODERATION_QUEUE, &bytes, Some(keypair), crate::io::Confirm::Accepted).await?;
        Ok(())
    }

    pub(crate) async fn read_mek_vault(
        &self, registry_key: &str,
    ) -> Result<Vec<MekVaultEntry>, ChatError> {
        let data = self.io
            .read_record(registry_key, REGISTRY_MEK_VAULT, false).await?;
        match data {
            Some(bytes) if !bytes.is_empty() => serde_json::from_slice(&bytes)
                .map_err(|e| ChatError::Deserialization(format!("mek vault: {e}"))),
            _ => Ok(Vec::new()),
        }
    }

    pub(crate) async fn write_mek_vault(
        &self, registry_key: &str, vault: &[MekVaultEntry], keypair: &[u8],
    ) -> Result<(), ChatError> {
        let bytes = serde_json::to_vec(vault)
            .map_err(|e| ChatError::Serialization(format!("mek vault: {e}")))?;
        self.io
            .write_record(registry_key, REGISTRY_MEK_VAULT, &bytes, Some(keypair), crate::io::Confirm::Accepted).await?;
        Ok(())
    }

    pub(crate) fn require_operator(
        &self, governance_key: &str,
    ) -> Result<rekindle_types::session_types::CommunityMembership, ChatError> {
        let meta = self.session_meta.read();
        let membership = meta.communities.get(governance_key).cloned()
            .ok_or_else(|| ChatError::NotMember { community: governance_key.into() })?;
        if !membership.is_operator {
            return Err(ChatError::InsufficientPermissions {
                action: "operator-only action".into(),
            });
        }
        Ok(membership)
    }

    pub(crate) fn require_registry_keypair(
        &self, registry_key: &str,
    ) -> Result<Vec<u8>, ChatError> {
        let reg_short = &registry_key[..12.min(registry_key.len())];
        self.vault.load_key(&rekindle_storage::keys::labels::registry_keypair(reg_short))?
            .ok_or_else(|| ChatError::InsufficientPermissions {
                action: "registry write (no keypair)".into(),
            })
    }

    // ── Public API ──────────────────────────────────────────────

    pub async fn list_members(
        &self, governance_key: &str,
    ) -> Result<Vec<MemberSummary>, ChatError> {
        let membership = {
            let meta = self.session_meta.read();
            meta.communities.get(governance_key).cloned()
                .ok_or_else(|| ChatError::NotMember { community: governance_key.into() })?
        };
        self.read_members(&membership.registry_key).await
    }

    pub fn list_communities(&self) -> Vec<CommunitySummary> {
        let meta = self.session_meta.read();
        meta.communities.values().map(|m| CommunitySummary {
            governance_key: m.governance_key.clone(),
            name: m.community_name.clone(),
            pseudonym: m.pseudonym_key.clone(),
            is_operator: m.is_operator,
        }).collect()
    }

    pub async fn pending_members(
        &self, governance_key: &str,
    ) -> Result<Vec<PendingJoinEntry>, ChatError> {
        let membership = self.require_operator(governance_key)?;
        self.read_moderation_queue(&membership.registry_key).await
    }

    pub async fn approve_member(
        &self, governance_key: &str, member_pseudonym: &str,
    ) -> Result<ApproveResult, ChatError> {
        let membership = self.require_operator(governance_key)?;
        let keypair = self.require_registry_keypair(&membership.registry_key)?;

        let mut queue = self.read_moderation_queue(&membership.registry_key).await?;
        let pending = queue.iter()
            .find(|p| p.requester_pseudonym_hex == member_pseudonym)
            .cloned()
            .ok_or_else(|| ChatError::Internal(format!(
                "no pending request from {member_pseudonym}"
            )))?;

        let mut members = self.read_members(&membership.registry_key).await?;
        let slot = members.iter().map(|m| m.subkey_index).max().map_or(1, |m| m + 1).max(1);

        members.push(MemberSummary {
            pseudonym_key: member_pseudonym.to_string(),
            display_name: pending.display_name.clone(),
            role_ids: Vec::new(),
            joined_at: timestamp_ms(),
            subkey_index: slot,
            onboarding_complete: true,
            timeout_until: None,
            profile_dht_key: Some(pending.profile_dht_key.clone()),
            x25519_pub: if pending.x25519_pub_hex.is_empty() { None } else { Some(pending.x25519_pub_hex.clone()) },
            channel_records: HashMap::new(),
        });
        let bytes = serde_json::to_vec(&members)
            .map_err(|e| ChatError::Serialization(format!("members: {e}")))?;
        self.io.write_record(
            &membership.registry_key, REGISTRY_MEMBER_INDEX, &bytes, Some(&keypair), crate::io::Confirm::Accepted,
        ).await?;

        queue.retain(|p| p.requester_pseudonym_hex != member_pseudonym);
        self.write_moderation_queue(&membership.registry_key, &queue, &keypair).await?;

        // Wrap MEKs for the approved member via ECDH
        if let Some((mek_bytes, gen)) = self.mek_cache.current(governance_key, "general") {
            if !pending.x25519_pub_hex.is_empty() {
                if let Some(recipient_pub) = hex::decode(&pending.x25519_pub_hex)
                    .ok()
                    .and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok())
                {
                    let operator_seed = self.io.pseudonym_seed(governance_key)?;
                    let operator_x25519 = blake3::derive_key("rekindle identity x25519 v1", &operator_seed);
                    let mek_wire = crate::crypto::mek::mek_to_wire(&mek_bytes, gen);

                    if let Ok(wrapped) = crate::crypto::mek::wrap_mek(&operator_x25519, &recipient_pub, &mek_wire) {
                        let mut vault = self.read_mek_vault(&membership.registry_key).await?;
                        if let Some(entry) = vault.iter_mut().find(|e| e.channel_id == "general") {
                            entry.copies.push(EncryptedMekCopy {
                                target_pseudonym: member_pseudonym.to_string(),
                                encrypted_mek: wrapped,
                            });
                        }
                        self.write_mek_vault(&membership.registry_key, &vault, &keypair).await?;
                    }
                }
            }
        }

        // Notify mesh peers of the new member
        let _ = self.io.broadcast_gossip_dedup(governance_key, rekindle_types::gossip_payload::GossipPayload::Control(
            rekindle_types::gossip_payload::ControlPayload::MemberJoined {
                pseudonym_key: member_pseudonym.into(),
                display_name: pending.display_name.clone(),
                role_ids: Vec::new(),
                status: "online".into(),
                route_blob: None,
            },
        )).await;

        // Direct notification to the joiner so they discover approval instantly
        let _ = self.io.send_gossip_direct(governance_key, member_pseudonym,
            rekindle_types::gossip_payload::GossipPayload::Control(
                rekindle_types::gossip_payload::ControlPayload::JoinAccepted {
                    mek_encrypted: Vec::new(),
                    mek_generation: 0,
                    member_registry_key: Some(membership.registry_key.clone()),
                    slot_index: Some(slot),
                    wrapped_slot_seed: None,
                },
            ),
        ).await;

        tracing::info!(member = member_pseudonym, slot, "member approved + notified");
        Ok(ApproveResult { slot, display_name: pending.display_name })
    }

    pub async fn reject_member(
        &self, governance_key: &str, member_pseudonym: &str, reason: &str,
    ) -> Result<(), ChatError> {
        let membership = self.require_operator(governance_key)?;
        let keypair = self.require_registry_keypair(&membership.registry_key)?;
        let mut queue = self.read_moderation_queue(&membership.registry_key).await?;
        queue.retain(|p| p.requester_pseudonym_hex != member_pseudonym);
        self.write_moderation_queue(&membership.registry_key, &queue, &keypair).await?;
        tracing::info!(member = member_pseudonym, reason, "member rejected");
        Ok(())
    }

    pub async fn transfer_ownership(
        &self, governance_key: &str, new_owner_pseudonym: &str,
    ) -> Result<(), ChatError> {
        let gov_short = &governance_key[..12.min(governance_key.len())];
        let gov_keypair = self.vault
            .load_key(&rekindle_storage::keys::labels::governance_keypair(gov_short))?
            .ok_or_else(|| ChatError::InsufficientPermissions {
                action: "governance write (no keypair)".into(),
            })?;

        let metadata_bytes = self.io
            .read_record(governance_key, MANIFEST_METADATA, true).await?
            .ok_or_else(|| ChatError::Internal("cannot read governance metadata".into()))?;
        let mut metadata: CommunityMetadata = serde_json::from_slice(&metadata_bytes)
            .map_err(|e| ChatError::Deserialization(format!("metadata: {e}")))?;

        let old_owner = metadata.owner_pseudonym.clone();
        metadata.owner_pseudonym = new_owner_pseudonym.to_string();
        metadata.operator_pseudonyms.retain(|p| p != &old_owner);
        if !metadata.operator_pseudonyms.contains(&new_owner_pseudonym.to_string()) {
            metadata.operator_pseudonyms.push(new_owner_pseudonym.to_string());
        }

        let updated_bytes = serde_json::to_vec(&metadata)
            .map_err(|e| ChatError::Serialization(format!("metadata: {e}")))?;
        self.io.write_record(
            governance_key, MANIFEST_METADATA, &updated_bytes, Some(&gov_keypair), crate::io::Confirm::Accepted,
        ).await?;

        {
            let mut meta = self.session_meta.write();
            if let Some(m) = meta.communities.get_mut(governance_key) {
                m.is_operator = false;
            }
        }
        let _ = self.io.broadcast_gossip_dedup(governance_key, rekindle_types::gossip_payload::GossipPayload::Control(
            rekindle_types::gossip_payload::ControlPayload::GovernanceUpdated {
                governance_key: governance_key.into(),
                subkey_index: MANIFEST_METADATA,
                lamport_ts: 0,
            },
        )).await;
        tracing::info!(old = %&old_owner[..12.min(old_owner.len())], new = %&new_owner_pseudonym[..12.min(new_owner_pseudonym.len())], "ownership transferred + notified");
        Ok(())
    }

    pub async fn remove_member(
        &self, governance_key: &str, pseudonym_key: &str,
    ) -> Result<(), ChatError> {
        let membership = self.require_operator(governance_key)?;
        let keypair = self.require_registry_keypair(&membership.registry_key)?;
        let mut members = self.read_members(&membership.registry_key).await?;
        members.retain(|m| m.pseudonym_key != pseudonym_key);
        let bytes = serde_json::to_vec(&members)
            .map_err(|e| ChatError::Serialization(format!("members: {e}")))?;
        self.io.write_record(
            &membership.registry_key, REGISTRY_MEMBER_INDEX, &bytes, Some(&keypair), crate::io::Confirm::Accepted,
        ).await?;
        tracing::info!(member = pseudonym_key, "member removed");
        Ok(())
    }

    // ── Join Inbox Processing ───────────────────────────────────

    /// Process pending join requests from the community's join inbox.
    /// Called periodically for each operator community.
    pub async fn process_join_inbox(
        &self, governance_key: &str,
    ) -> Result<u32, ChatError> {
        let Ok(membership) = self.require_operator(governance_key) else { return Ok(0) };
        let Ok(keypair) = self.require_registry_keypair(&membership.registry_key) else { return Ok(0) };

        // Read governance metadata for inbox key and join policy
        let metadata_bytes = match self.io
            .read_record(governance_key, MANIFEST_METADATA, true).await?
        {
            Some(b) if !b.is_empty() => b,
            _ => return Ok(0),
        };
        let metadata: CommunityMetadata = serde_json::from_slice(&metadata_bytes)
            .map_err(|e| ChatError::Deserialization(format!("metadata: {e}")))?;

        if metadata.join_inbox_key.is_empty() {
            return Ok(0);
        }

        // Inspect inbox for populated subkeys
        let subkeys: Vec<u32> = (0..32).collect();
        let seqs = self.io
            .inspect_record(&metadata.join_inbox_key, &subkeys).await?;

        let mut all_pending = Vec::new();
        for (i, seq) in seqs.iter().enumerate() {
            if seq.is_none() { continue; }
            let data = self.io
                .read_record(&metadata.join_inbox_key, u32::try_from(i).unwrap_or(0), true).await?;
            if let Some(bytes) = data {
                if bytes.is_empty() || bytes == b"[]" { continue; }
                if let Ok(entries) = serde_json::from_slice::<Vec<PendingJoinEntry>>(&bytes) {
                    all_pending.extend(entries);
                }
            }
        }

        if all_pending.is_empty() { return Ok(0); }

        let bans = self.read_bans(governance_key).await?;
        let mut members = self.read_members(&membership.registry_key).await?;
        let mut new_count = 0u32;

        // Process leave entries first
        let left: Vec<String> = all_pending.iter()
            .filter(|p| matches!(p.status, PendingJoinStatus::Left { .. }))
            .map(|p| p.requester_pseudonym_hex.clone())
            .collect();
        if !left.is_empty() {
            let before = members.len();
            for pseudonym in &left {
                members.retain(|m| m.pseudonym_key != *pseudonym);
            }
            if members.len() < before {
                let bytes = serde_json::to_vec(&members)
                    .map_err(|e| ChatError::Serialization(format!("members: {e}")))?;
                self.io.write_record(
                    &membership.registry_key, REGISTRY_MEMBER_INDEX, &bytes, Some(&keypair), crate::io::Confirm::Accepted,
                ).await?;

                // Remove vault copies and rekey
                let mut vault = self.read_mek_vault(&membership.registry_key).await?;
                for entry in &mut vault {
                    entry.copies.retain(|c| !left.contains(&c.target_pseudonym));
                }
                self.write_mek_vault(&membership.registry_key, &vault, &keypair).await?;
                self.rekey_all_channels(governance_key, &membership.registry_key, &members).await;
                tracing::info!(removed = before - members.len(), "processed leave entries + rekeyed");
            }
        }

        // Process join entries
        for req in &all_pending {
            if matches!(req.status, PendingJoinStatus::Left { .. }) { continue; }

            // Verify signature
            if !req.signature_hex.is_empty() {
                let sig_ok = verify_join_signature(req);
                if !sig_ok {
                    tracing::warn!(requester = %&req.requester_pseudonym_hex[..16.min(req.requester_pseudonym_hex.len())], "SIGNATURE FAILED — skipping");
                    continue;
                }
            }

            // Skip banned
            if bans.iter().any(|b| b.pseudonym_key == req.requester_pseudonym_hex) { continue; }
            // Skip already member
            if members.iter().any(|m| m.pseudonym_key == req.requester_pseudonym_hex) { continue; }

            // Check join policy
            match metadata.join_policy {
                rekindle_types::dht_types::JoinPolicy::AutoAllow => {}
                rekindle_types::dht_types::JoinPolicy::WaitingRoom => {
                    let mut queue = self.read_moderation_queue(&membership.registry_key).await?;
                    if !queue.iter().any(|p| p.requester_pseudonym_hex == req.requester_pseudonym_hex) {
                        queue.push(req.clone());
                        self.write_moderation_queue(&membership.registry_key, &queue, &keypair).await?;
                    }
                    continue;
                }
                rekindle_types::dht_types::JoinPolicy::InviteOnly => {
                    if req.invite_code_hash.is_none() { continue; }
                }
            }

            // Register member
            let slot = members.iter().map(|m| m.subkey_index).max().map_or(1, |m| m + 1).max(1);
            members.push(MemberSummary {
                pseudonym_key: req.requester_pseudonym_hex.clone(),
                display_name: req.display_name.clone(),
                role_ids: Vec::new(),
                joined_at: timestamp_ms(),
                subkey_index: slot,
                onboarding_complete: true,
                timeout_until: None,
                x25519_pub: if req.x25519_pub_hex.is_empty() { None } else { Some(req.x25519_pub_hex.clone()) },
                profile_dht_key: Some(req.profile_dht_key.clone()),
                channel_records: HashMap::new(),
            });
            new_count += 1;
            tracing::info!(member = %req.display_name, slot, "member registered from inbox");
        }

        if new_count == 0 { return Ok(0); }

        // Write updated member index
        let bytes = serde_json::to_vec(&members)
            .map_err(|e| ChatError::Serialization(format!("members: {e}")))?;
        self.io.write_record(
            &membership.registry_key, REGISTRY_MEMBER_INDEX, &bytes, Some(&keypair), crate::io::Confirm::Accepted,
        ).await?;

        // Wrap MEKs for new members via ECDH
        let operator_seed = self.io.pseudonym_seed(governance_key)?;
        let operator_x25519 = blake3::derive_key("rekindle identity x25519 v1", &operator_seed);
        let mut vault = self.read_mek_vault(&membership.registry_key).await.unwrap_or_default();
        for req in &all_pending {
            if bans.iter().any(|b| b.pseudonym_key == req.requester_pseudonym_hex) { continue; }
            if matches!(req.status, PendingJoinStatus::Left { .. }) { continue; }
            if req.x25519_pub_hex.is_empty() { continue; }
            let Some(recipient_pub) = hex::decode(&req.x25519_pub_hex)
                .ok()
                .and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok()) else { continue };
            if let Some((mek_bytes, gen)) = self.mek_cache.current(governance_key, "general") {
                let mek_wire = crate::crypto::mek::mek_to_wire(&mek_bytes, gen);
                if let Ok(wrapped) = crate::crypto::mek::wrap_mek(&operator_x25519, &recipient_pub, &mek_wire) {
                    if let Some(entry) = vault.iter_mut().find(|e| e.channel_id == "general") {
                        entry.copies.push(EncryptedMekCopy {
                            target_pseudonym: req.requester_pseudonym_hex.clone(),
                            encrypted_mek: wrapped,
                        });
                    }
                }
            }
        }
        self.write_mek_vault(&membership.registry_key, &vault, &keypair).await?;

        tracing::info!(new_members = new_count, total = members.len(), "inbox processing complete");
        Ok(new_count)
    }

    // ── Leave Handler ───────────────────────────────────────────

    /// Handle a member leaving — remove from index, remove vault copies, rekey.
    pub async fn handle_member_leave(
        &self, governance_key: &str, leaving_pseudonym: &str,
    ) -> Result<(), ChatError> {
        let membership = self.require_operator(governance_key)?;
        let keypair = self.require_registry_keypair(&membership.registry_key)?;

        // Remove from member index
        let mut members = self.read_members(&membership.registry_key).await?;
        let before = members.len();
        members.retain(|m| m.pseudonym_key != leaving_pseudonym);
        if members.len() < before {
            let bytes = serde_json::to_vec(&members)
                .map_err(|e| ChatError::Serialization(format!("members: {e}")))?;
            self.io.write_record(
                &membership.registry_key, REGISTRY_MEMBER_INDEX, &bytes, Some(&keypair), crate::io::Confirm::Accepted,
            ).await?;
        }

        // Remove vault copies for the leaving member
        let mut vault = self.read_mek_vault(&membership.registry_key).await.unwrap_or_default();
        for entry in &mut vault {
            entry.copies.retain(|c| c.target_pseudonym != leaving_pseudonym);
        }
        self.write_mek_vault(&membership.registry_key, &vault, &keypair).await?;

        // Rekey all channels for forward secrecy
        self.rekey_all_channels(governance_key, &membership.registry_key, &members).await;

        let _ = self.io.broadcast_gossip_dedup(governance_key, rekindle_types::gossip_payload::GossipPayload::Control(
            rekindle_types::gossip_payload::ControlPayload::MemberRemoved {
                pseudonym_key: leaving_pseudonym.into(),
            },
        )).await;
        tracing::info!(
            member = %&leaving_pseudonym[..12.min(leaving_pseudonym.len())],
            remaining = members.len(),
            "leave processed + rekeyed + notified",
        );
        Ok(())
    }

    // read_bans is defined in governance.rs as pub(crate) on CommunityService
}

fn verify_join_signature(req: &PendingJoinEntry) -> bool {
    let sig_bytes: [u8; 64] = match hex::decode(&req.signature_hex)
        .ok()
        .and_then(|b| b.try_into().ok())
    {
        Some(b) => b,
        None => return false,
    };
    let pub_bytes: [u8; 32] = match hex::decode(&req.requester_pseudonym_hex)
        .ok()
        .and_then(|b| b.try_into().ok())
    {
        Some(b) => b,
        None => return false,
    };
    rekindle_ratchet::crypto::sign::verify_ec_prekey(
        &pub_bytes,
        &req.signature_content(),
        &sig_bytes,
    ).is_ok()
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CommunitySummary {
    pub governance_key: String,
    pub name: String,
    pub pseudonym: String,
    pub is_operator: bool,
}

pub struct ApproveResult {
    pub slot: u32,
    pub display_name: String,
}
