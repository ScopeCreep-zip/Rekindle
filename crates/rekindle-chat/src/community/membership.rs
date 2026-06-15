//! Member registry operations — inspect-based member discovery, operator
//! approval, MEK wrapping, moderation queue, leave handling.
//!
//! With o_cnt=0 SMPL registry, each member writes their own MemberSummary
//! to their slot. The member list is discovered via inspect (populated
//! slots), not a centralized JSON array. MEK vault lives in a separate
//! DFLT record pointed to from CommunityMetadata.mek_vault_key.

use std::collections::HashMap;

use rekindle_types::dht_types::{
    CommunityMetadata, EncryptedMekCopy, MemberSummary, MekVaultEntry,
    PendingJoinEntry, PendingJoinStatus,
    MANIFEST_METADATA,
};

use crate::time::timestamp_ms;
use crate::ChatError;
use super::CommunityService;

impl CommunityService {
    // ── Internal helpers ─────────────────────────────────────────

    /// Read all members by inspecting the registry for populated slots
    /// and reading each one. With o_cnt=0 SMPL, each member writes their
    /// own MemberSummary to their slot.
    pub(crate) async fn read_members(
        &self, registry_key: &str,
    ) -> Result<Vec<MemberSummary>, ChatError> {
        let all_subkeys: Vec<u32> = (0..255u32).collect();
        let seqs = self.io.open_and_inspect(registry_key, &all_subkeys).await?.local_seqs;

        let mut members = Vec::new();
        for (i, seq) in seqs.iter().enumerate() {
            if seq.is_none() { continue; }
            let subkey = i as u32;
            let data = match self.io.open_and_read(registry_key, subkey, false).await {
                Ok(Some(d)) if !d.is_empty() => d,
                _ => continue,
            };
            // Try parsing as MemberSummary (new format: one member per slot)
            if let Ok(member) = serde_json::from_slice::<MemberSummary>(&data) {
                members.push(member);
            }
            // Also try Vec<MemberSummary> for backwards compat with old slot 0 format
            else if let Ok(vec) = serde_json::from_slice::<Vec<MemberSummary>>(&data) {
                members.extend(vec);
            }
        }
        Ok(members)
    }

    /// Read MEK vault from the separate DFLT record.
    pub(crate) async fn read_mek_vault_from_metadata(
        &self, governance_key: &str,
    ) -> Result<Vec<MekVaultEntry>, ChatError> {
        let metadata = self.read_metadata(governance_key).await?;
        if metadata.mek_vault_key.is_empty() {
            return Ok(Vec::new());
        }
        let data = self.io.open_and_read(&metadata.mek_vault_key, 0, false).await?;
        match data {
            Some(bytes) if !bytes.is_empty() => serde_json::from_slice(&bytes)
                .map_err(|e| ChatError::Deserialization(format!("mek vault: {e}"))),
            _ => Ok(Vec::new()),
        }
    }

    /// Write MEK vault to the separate DFLT record.
    pub(crate) async fn write_mek_vault_to_metadata(
        &self, governance_key: &str, vault: &[MekVaultEntry],
    ) -> Result<(), ChatError> {
        let metadata = self.read_metadata(governance_key).await?;
        if metadata.mek_vault_key.is_empty() {
            return Err(ChatError::Internal("no mek_vault_key in metadata".into()));
        }
        let vault_keypair = hex::decode(&metadata.mek_vault_keypair_hex)
            .map_err(|e| ChatError::Internal(format!("mek vault keypair hex: {e}")))?;
        let bytes = serde_json::to_vec(vault)
            .map_err(|e| ChatError::Serialization(format!("mek vault: {e}")))?;
        self.io.open_and_write(
            &metadata.mek_vault_key, 0, &bytes,
            Some(&vault_keypair), crate::io::Confirm::Accepted,
        ).await?;
        Ok(())
    }

    // Keep old read_mek_vault/write_mek_vault signatures for callers that
    // still use registry_key — redirect to metadata-based versions.
    pub(crate) async fn read_mek_vault(
        &self, _registry_key: &str,
    ) -> Result<Vec<MekVaultEntry>, ChatError> {
        // Find the governance key for this registry
        let gov_key = {
            let meta = self.session_meta.read();
            meta.communities.values()
                .find(|m| m.registry_key == _registry_key)
                .map(|m| m.governance_key.clone())
                .unwrap_or_default()
        };
        if gov_key.is_empty() {
            return Ok(Vec::new());
        }
        self.read_mek_vault_from_metadata(&gov_key).await
    }

    pub(crate) async fn write_mek_vault(
        &self, _registry_key: &str, vault: &[MekVaultEntry], _keypair: &[u8],
    ) -> Result<(), ChatError> {
        let gov_key = {
            let meta = self.session_meta.read();
            meta.communities.values()
                .find(|m| m.registry_key == _registry_key)
                .map(|m| m.governance_key.clone())
                .unwrap_or_default()
        };
        if gov_key.is_empty() {
            return Err(ChatError::Internal("no community for registry key".into()));
        }
        self.write_mek_vault_to_metadata(&gov_key, vault).await
    }

    /// Read pending join requests from the join inbox.
    /// Replaces the old moderation queue subkey — pending requests live
    /// in the join inbox itself, not a separate registry subkey.
    pub(crate) async fn read_pending_from_inbox(
        &self, governance_key: &str,
    ) -> Result<Vec<PendingJoinEntry>, ChatError> {
        let metadata = self.read_metadata(governance_key).await?;
        if metadata.join_inbox_key.is_empty() { return Ok(Vec::new()); }
        let subkeys: Vec<u32> = (0..32).collect();
        let seqs = self.io.open_and_inspect(&metadata.join_inbox_key, &subkeys).await?.local_seqs;
        let mut all = Vec::new();
        for (i, seq) in seqs.iter().enumerate() {
            if seq.is_none() { continue; }
            let data = self.io
                .open_and_read(&metadata.join_inbox_key, u32::try_from(i).unwrap_or(0), true).await?;
            if let Some(bytes) = data {
                if bytes.is_empty() || bytes == b"[]" { continue; }
                if let Ok(entries) = serde_json::from_slice::<Vec<PendingJoinEntry>>(&bytes) {
                    all.extend(entries);
                }
            }
        }
        Ok(all)
    }

    /// Derive the operator's slot keypair from slot_seed for SMPL writes.
    pub(crate) fn require_operator_slot_keypair(
        &self, governance_key: &str,
    ) -> Result<Vec<u8>, ChatError> {
        let membership = {
            let meta = self.session_meta.read();
            meta.communities.get(governance_key).cloned()
                .ok_or_else(|| ChatError::NotMember { community: governance_key.into() })?
        };
        let slot_seed = membership.slot_seed.as_ref()
            .and_then(|h| hex::decode(h).ok())
            .and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok())
            .ok_or_else(|| ChatError::Internal("slot_seed not available".into()))?;

        let kp = rekindle_identity::derive_slot_keypair(&slot_seed, membership.slot_index)
            .map_err(|e| ChatError::Internal(format!("slot keypair: {e}")))?;
        let mut buf = vec![0u8; 64];
        buf[..32].copy_from_slice(&kp.public_key_bytes());
        let mut ikm = Vec::with_capacity(36);
        ikm.extend_from_slice(&slot_seed);
        ikm.extend_from_slice(&membership.slot_index.to_le_bytes());
        let seed = blake3::derive_key(rekindle_identity::derivation_tags::SLOT_KEYPAIR, &ikm);
        buf[32..].copy_from_slice(&seed);
        Ok(buf)
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

    // Keep for callers that haven't migrated yet — redirects to slot keypair
    pub(crate) fn require_registry_keypair(
        &self, _registry_key: &str,
    ) -> Result<Vec<u8>, ChatError> {
        let gov_key = {
            let meta = self.session_meta.read();
            meta.communities.values()
                .find(|m| m.registry_key == _registry_key)
                .map(|m| m.governance_key.clone())
                .unwrap_or_default()
        };
        if gov_key.is_empty() {
            return Err(ChatError::InsufficientPermissions {
                action: "registry write (no community)".into(),
            });
        }
        self.require_operator_slot_keypair(&gov_key)
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
        let _membership = self.require_operator(governance_key)?;
        self.read_pending_from_inbox(governance_key).await
    }

    pub async fn approve_member(
        &self, governance_key: &str, member_pseudonym: &str,
    ) -> Result<ApproveResult, ChatError> {
        let membership = self.require_operator(governance_key)?;

        let all_pending = self.read_pending_from_inbox(governance_key).await?;
        let pending = all_pending.iter()
            .find(|p| p.requester_pseudonym_hex == member_pseudonym)
            .cloned()
            .ok_or_else(|| ChatError::Internal(format!(
                "no pending request from {member_pseudonym}"
            )))?;

        let members = self.read_members(&membership.registry_key).await?;

        // max_members enforcement
        let metadata = self.read_metadata(governance_key).await?;
        if members.len() >= metadata.max_members as usize {
            return Err(ChatError::CommunityFull {
                current: members.len(),
                max: metadata.max_members,
            });
        }

        // Find first empty slot via inspect
        let all_subkeys: Vec<u32> = (0..255u32).collect();
        let seqs = self.io.open_and_inspect(&membership.registry_key, &all_subkeys).await?.local_seqs;
        let slot = seqs.iter().enumerate()
            .find(|(_, seq)| seq.is_none())
            .map(|(i, _)| i as u32)
            .ok_or_else(|| ChatError::CommunityFull { current: members.len(), max: 255 })?;

        // Write new member's MemberSummary to their slot using the operator's
        // slot keypair. Note: with o_cnt=0, the operator can only write to
        // their OWN slot. The new member's slot must be written by someone
        // who holds that slot's keypair. Since everyone derives all keypairs
        // from the shared slot_seed, the operator CAN derive the new member's
        // keypair and write to their slot.
        let new_member_slot_seed = membership.slot_seed.as_ref()
            .and_then(|h| hex::decode(h).ok())
            .and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok())
            .ok_or_else(|| ChatError::Internal("slot_seed not available".into()))?;

        let new_member_kp = rekindle_identity::derive_slot_keypair(&new_member_slot_seed, slot)
            .map_err(|e| ChatError::Internal(format!("new member slot keypair: {e}")))?;
        let new_member_kp_bytes = {
            let mut buf = [0u8; 64];
            buf[..32].copy_from_slice(&new_member_kp.public_key_bytes());
            let mut ikm = Vec::with_capacity(36);
            ikm.extend_from_slice(&new_member_slot_seed);
            ikm.extend_from_slice(&slot.to_le_bytes());
            let seed = blake3::derive_key(rekindle_identity::derivation_tags::SLOT_KEYPAIR, &ikm);
            buf[32..].copy_from_slice(&seed);
            buf
        };

        let new_member = MemberSummary {
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
            signature: Vec::new(),
        };
        let member_bytes = serde_json::to_vec(&new_member)
            .map_err(|e| ChatError::Serialization(format!("member: {e}")))?;
        self.io.open_and_write(
            &membership.registry_key, slot, &member_bytes,
            Some(&new_member_kp_bytes), crate::io::Confirm::Accepted,
        ).await?;

        // Wrap MEKs + slot_seed for the approved member
        if !pending.x25519_pub_hex.is_empty() {
            if let Some(recipient_dh_key) = rekindle_identity::DhKey::from_hex(&pending.x25519_pub_hex).ok() {
                let channels = self.read_channels(governance_key).await.unwrap_or_default();
                let operator_seed = self.io.pseudonym_seed(governance_key)?;
                let operator_x25519 = rekindle_identity::x25519_seed_from(&operator_seed);
                let operator_x25519_pub = rekindle_identity::x25519_public_from_raw_seed(&operator_x25519)
                    .map_err(|e| ChatError::Internal(format!("operator DhKey: {e}")))?;
                let mut vault = self.read_mek_vault_from_metadata(governance_key).await?;

                for ch in &channels {
                    if let Some((mek_bytes, gen)) = self.mek_cache.current(governance_key, &ch.id) {
                        let mek_wire = crate::crypto::mek::mek_to_wire(&mek_bytes, gen);
                        if let Ok(wrapped) = crate::crypto::mek::wrap_mek(&operator_x25519, &recipient_dh_key, &mek_wire) {
                            if let Some(entry) = vault.iter_mut().find(|e| e.channel_id == ch.id) {
                                entry.copies.push(EncryptedMekCopy {
                                    target_pseudonym: member_pseudonym.to_string(),
                                    encrypted_mek: wrapped,
                                });
                                entry.rotator_x25519_pub = Some(operator_x25519_pub.clone());
                            }
                        }
                    }
                }
                // Wrap slot_seed for the new member
                let wrapped_ss = crate::crypto::mek::wrap_mek(&operator_x25519, &recipient_dh_key, &new_member_slot_seed)?;
                if let Some(entry) = vault.first_mut() {
                    entry.wrapped_slot_seeds.push(rekindle_types::dht_types::WrappedSlotSeed {
                        target_pseudonym: member_pseudonym.to_string(),
                        encrypted_slot_seed: wrapped_ss,
                    });
                }

                self.write_mek_vault_to_metadata(governance_key, &vault).await?;
            }
        }

        // Discover route + gossip MemberJoined
        let mut member_route_blob: Option<Vec<u8>> = None;
        if !pending.profile_dht_key.is_empty() {
            let _ = self.io.open_record(&pending.profile_dht_key, None).await;
            self.io.transport().register_peer_profile(member_pseudonym, &pending.profile_dht_key);
            let mesh_info = crate::io::route::MeshPeerInfo {
                community_id: governance_key,
                pseudonym: member_pseudonym,
                status: "online",
                my_pseudonym: &membership.pseudonym_key,
            };
            if let Ok(true) = self.io.discover_peer_route(member_pseudonym, &pending.profile_dht_key, Some(mesh_info)).await {
                if let Ok(Some(blob)) = self.io.open_and_read(&pending.profile_dht_key, rekindle_types::dht_types::PROFILE_SUBKEY_ROUTE_BLOB, false).await {
                    if !blob.is_empty() { member_route_blob = Some(blob); }
                }
            }
        }

        let _ = self.io.broadcast_gossip_dedup(governance_key, rekindle_types::gossip_payload::GossipPayload::Control(
            rekindle_types::gossip_payload::ControlPayload::MemberJoined {
                pseudonym_key: member_pseudonym.into(),
                display_name: pending.display_name.clone(),
                role_ids: Vec::new(),
                status: "online".into(),
                route_blob: member_route_blob,
            },
        )).await;

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

        self.append_audit_entry(governance_key, "approve", &membership.pseudonym_key, Some(member_pseudonym), None).await;

        self.discover_community_member_routes(
            governance_key, &membership.pseudonym_key, &membership.registry_key,
        ).await;

        tracing::info!(member = member_pseudonym, slot, "member approved");
        Ok(ApproveResult { slot, display_name: pending.display_name })
    }

    pub async fn reject_member(
        &self, governance_key: &str, member_pseudonym: &str, reason: &str,
    ) -> Result<(), ChatError> {
        let _membership = self.require_operator(governance_key)?;
        self.append_audit_entry(governance_key, "reject", "", Some(member_pseudonym), Some(reason)).await;
        tracing::info!(member = member_pseudonym, reason, "member rejected");
        Ok(())
    }

    pub async fn transfer_ownership(
        &self, governance_key: &str, new_owner_pseudonym: &str,
    ) -> Result<(), ChatError> {
        self.require_operator(governance_key)?;

        let metadata_raw = self.read_verified_governance(governance_key, MANIFEST_METADATA).await?
            .ok_or_else(|| ChatError::Internal("cannot read governance metadata".into()))?;
        let mut metadata: CommunityMetadata = serde_json::from_slice(&metadata_raw)
            .map_err(|e| ChatError::Deserialization(format!("metadata: {e}")))?;

        let old_owner = metadata.owner_pseudonym.clone();
        metadata.owner_pseudonym = new_owner_pseudonym.to_string();
        metadata.operator_pseudonyms.retain(|p| p != &old_owner);
        if !metadata.operator_pseudonyms.contains(&new_owner_pseudonym.to_string()) {
            metadata.operator_pseudonyms.push(new_owner_pseudonym.to_string());
        }

        let updated_bytes = serde_json::to_vec(&metadata)
            .map_err(|e| ChatError::Serialization(format!("metadata: {e}")))?;
        self.write_signed_governance(governance_key, MANIFEST_METADATA, &updated_bytes).await?;

        {
            let mut meta = self.session_meta.write();
            if let Some(m) = meta.communities.get_mut(governance_key) {
                m.is_operator = false;
            }
        }
        tracing::info!(old = %&old_owner[..12.min(old_owner.len())], new = %&new_owner_pseudonym[..12.min(new_owner_pseudonym.len())], "ownership transferred");
        Ok(())
    }

    pub async fn remove_member(
        &self, governance_key: &str, pseudonym_key: &str,
    ) -> Result<(), ChatError> {
        let membership = self.require_operator(governance_key)?;
        // Find the member's slot and clear it
        let members = self.read_members(&membership.registry_key).await?;
        if let Some(member) = members.iter().find(|m| m.pseudonym_key == pseudonym_key) {
            let slot_seed = membership.slot_seed.as_ref()
                .and_then(|h| hex::decode(h).ok())
                .and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok())
                .ok_or_else(|| ChatError::Internal("slot_seed not available".into()))?;
            let kp = rekindle_identity::derive_slot_keypair(&slot_seed, member.subkey_index)
                .map_err(|e| ChatError::Internal(format!("slot keypair: {e}")))?;
            let kp_bytes = {
                let mut buf = [0u8; 64];
                buf[..32].copy_from_slice(&kp.public_key_bytes());
                let mut ikm = Vec::with_capacity(36);
                ikm.extend_from_slice(&slot_seed);
                ikm.extend_from_slice(&member.subkey_index.to_le_bytes());
                let seed = blake3::derive_key(rekindle_identity::derivation_tags::SLOT_KEYPAIR, &ikm);
                buf[32..].copy_from_slice(&seed);
                buf
            };
            // Write empty bytes to clear the slot
            self.io.open_and_write(
                &membership.registry_key, member.subkey_index, b"",
                Some(&kp_bytes), crate::io::Confirm::Accepted,
            ).await?;
        }
        tracing::info!(member = pseudonym_key, "member removed");
        Ok(())
    }

    // ── Join Inbox Processing ───────────────────────────────────

    pub async fn process_join_inbox(
        &self, governance_key: &str,
    ) -> Result<u32, ChatError> {
        let Ok(membership) = self.require_operator(governance_key) else { return Ok(0) };

        let metadata_bytes = match self.io
            .open_and_read(governance_key, MANIFEST_METADATA, true).await?
        {
            Some(b) if !b.is_empty() => b,
            _ => return Ok(0),
        };
        let metadata: CommunityMetadata = serde_json::from_slice(&metadata_bytes)
            .map_err(|e| ChatError::Deserialization(format!("metadata: {e}")))?;

        if metadata.join_inbox_key.is_empty() { return Ok(0); }

        let subkeys: Vec<u32> = (0..32).collect();
        let seqs = self.io.open_and_inspect(&metadata.join_inbox_key, &subkeys).await?.local_seqs;

        let mut all_pending = Vec::new();
        for (i, seq) in seqs.iter().enumerate() {
            if seq.is_none() { continue; }
            let data = self.io
                .open_and_read(&metadata.join_inbox_key, u32::try_from(i).unwrap_or(0), true).await?;
            if let Some(bytes) = data {
                if bytes.is_empty() || bytes == b"[]" { continue; }
                if let Ok(entries) = serde_json::from_slice::<Vec<PendingJoinEntry>>(&bytes) {
                    all_pending.extend(entries);
                }
            }
        }

        if all_pending.is_empty() { return Ok(0); }

        let bans = self.read_bans(governance_key).await?;
        let members = self.read_members(&membership.registry_key).await?;
        let mut new_count = 0u32;

        // Find empty slots via inspect
        let reg_subkeys: Vec<u32> = (0..255u32).collect();
        let reg_seqs = self.io.open_and_inspect(&membership.registry_key, &reg_subkeys).await?.local_seqs;
        let mut empty_slots: Vec<u32> = reg_seqs.iter().enumerate()
            .filter(|(_, seq)| seq.is_none())
            .map(|(i, _)| i as u32)
            .collect();

        let slot_seed = membership.slot_seed.as_ref()
            .and_then(|h| hex::decode(h).ok())
            .and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok())
            .ok_or_else(|| ChatError::Internal("slot_seed not available for inbox processing".into()))?;

        let operator_seed = self.io.pseudonym_seed(governance_key)?;
        let operator_x25519 = rekindle_identity::x25519_seed_from(&operator_seed);
        let operator_x25519_pub = rekindle_identity::x25519_public_from_raw_seed(&operator_x25519)
            .map_err(|e| ChatError::Internal(format!("operator DhKey: {e}")))?;
        let mut vault = self.read_mek_vault_from_metadata(governance_key).await.unwrap_or_default();
        let channels = self.read_channels(governance_key).await.unwrap_or_default();

        for req in &all_pending {
            if matches!(req.status, PendingJoinStatus::Left { .. }) { continue; }
            if !req.signature_hex.is_empty() && !verify_join_signature(req) { continue; }
            if bans.iter().any(|b| b.pseudonym_key == req.requester_pseudonym_hex) { continue; }
            if members.iter().any(|m| m.pseudonym_key == req.requester_pseudonym_hex) { continue; }

            match metadata.join_policy {
                rekindle_types::dht_types::JoinPolicy::AutoAllow => {}
                rekindle_types::dht_types::JoinPolicy::WaitingRoom => { continue; }
                rekindle_types::dht_types::JoinPolicy::InviteOnly => {
                    if req.invite_code_hash.is_none() { continue; }
                }
            }

            let Some(slot) = empty_slots.first().copied() else {
                tracing::warn!("no empty slots — community full");
                break;
            };
            empty_slots.remove(0);

            // Derive the new member's slot keypair and write their MemberSummary
            let member_kp = rekindle_identity::derive_slot_keypair(&slot_seed, slot)
                .map_err(|e| ChatError::Internal(format!("member slot keypair: {e}")))?;
            let member_kp_bytes = {
                let mut buf = [0u8; 64];
                buf[..32].copy_from_slice(&member_kp.public_key_bytes());
                let mut ikm = Vec::with_capacity(36);
                ikm.extend_from_slice(&slot_seed);
                ikm.extend_from_slice(&slot.to_le_bytes());
                let seed = blake3::derive_key(rekindle_identity::derivation_tags::SLOT_KEYPAIR, &ikm);
                buf[32..].copy_from_slice(&seed);
                buf
            };

            let new_member = MemberSummary {
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
                signature: Vec::new(),
            };
            let member_bytes = serde_json::to_vec(&new_member)
                .map_err(|e| ChatError::Serialization(format!("member: {e}")))?;
            self.io.open_and_write(
                &membership.registry_key, slot, &member_bytes,
                Some(&member_kp_bytes), crate::io::Confirm::Accepted,
            ).await?;

            // Wrap MEKs + slot_seed for new member
            if !req.x25519_pub_hex.is_empty() {
                if let Some(recipient_dh_key) = rekindle_identity::DhKey::from_hex(&req.x25519_pub_hex).ok() {
                    for ch in &channels {
                        if let Some((mek_bytes, gen)) = self.mek_cache.current(governance_key, &ch.id) {
                            let mek_wire = crate::crypto::mek::mek_to_wire(&mek_bytes, gen);
                            if let Ok(wrapped) = crate::crypto::mek::wrap_mek(&operator_x25519, &recipient_dh_key, &mek_wire) {
                                if let Some(entry) = vault.iter_mut().find(|e| e.channel_id == ch.id) {
                                    entry.copies.push(EncryptedMekCopy {
                                        target_pseudonym: req.requester_pseudonym_hex.clone(),
                                        encrypted_mek: wrapped,
                                    });
                                    entry.rotator_x25519_pub = Some(operator_x25519_pub.clone());
                                }
                            }
                        }
                    }
                    // Wrap slot_seed
                    if let Ok(wrapped_ss) = crate::crypto::mek::wrap_mek(&operator_x25519, &recipient_dh_key, &slot_seed) {
                        if let Some(entry) = vault.first_mut() {
                            entry.wrapped_slot_seeds.push(rekindle_types::dht_types::WrappedSlotSeed {
                                target_pseudonym: req.requester_pseudonym_hex.clone(),
                                encrypted_slot_seed: wrapped_ss,
                            });
                        }
                    }
                }
            }

            new_count += 1;
            tracing::info!(member = %req.display_name, slot, "member registered from inbox");
        }

        if new_count > 0 {
            self.write_mek_vault_to_metadata(governance_key, &vault).await?;
            self.discover_community_member_routes(
                governance_key, &membership.pseudonym_key, &membership.registry_key,
            ).await;
        }

        tracing::info!(new_members = new_count, "inbox processing complete");
        Ok(new_count)
    }

    // ── Leave Handler ───────────────────────────────────────────

    pub async fn handle_member_leave(
        &self, governance_key: &str, leaving_pseudonym: &str,
    ) -> Result<(), ChatError> {
        let membership = self.require_operator(governance_key)?;

        // Find the member's slot and clear it
        let members = self.read_members(&membership.registry_key).await?;
        if let Some(member) = members.iter().find(|m| m.pseudonym_key == leaving_pseudonym) {
            let slot_seed = membership.slot_seed.as_ref()
                .and_then(|h| hex::decode(h).ok())
                .and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok())
                .ok_or_else(|| ChatError::Internal("slot_seed not available".into()))?;
            let kp = rekindle_identity::derive_slot_keypair(&slot_seed, member.subkey_index)
                .map_err(|e| ChatError::Internal(format!("slot keypair: {e}")))?;
            let kp_bytes = {
                let mut buf = [0u8; 64];
                buf[..32].copy_from_slice(&kp.public_key_bytes());
                let mut ikm = Vec::with_capacity(36);
                ikm.extend_from_slice(&slot_seed);
                ikm.extend_from_slice(&member.subkey_index.to_le_bytes());
                let seed = blake3::derive_key(rekindle_identity::derivation_tags::SLOT_KEYPAIR, &ikm);
                buf[32..].copy_from_slice(&seed);
                buf
            };
            self.io.open_and_write(
                &membership.registry_key, member.subkey_index, b"",
                Some(&kp_bytes), crate::io::Confirm::Accepted,
            ).await?;
        }

        // Remove vault copies + rekey
        let mut vault = self.read_mek_vault_from_metadata(governance_key).await.unwrap_or_default();
        for entry in &mut vault {
            entry.copies.retain(|c| c.target_pseudonym != leaving_pseudonym);
        }
        self.write_mek_vault_to_metadata(governance_key, &vault).await?;

        let remaining = self.read_members(&membership.registry_key).await?;
        self.rekey_all_channels(governance_key, &membership.registry_key, &remaining).await;

        let _ = self.io.broadcast_gossip_dedup(governance_key, rekindle_types::gossip_payload::GossipPayload::Control(
            rekindle_types::gossip_payload::ControlPayload::MemberRemoved {
                pseudonym_key: leaving_pseudonym.into(),
            },
        )).await;
        tracing::info!(
            member = %&leaving_pseudonym[..12.min(leaving_pseudonym.len())],
            remaining = remaining.len(),
            "leave processed + rekeyed + notified",
        );
        Ok(())
    }
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
    rekindle_identity::verify_ec_prekey(
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
