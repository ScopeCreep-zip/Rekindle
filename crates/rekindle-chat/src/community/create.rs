//! Community creation — governance record, registry, shared SMPL channel
//! record, MEK vault (separate DFLT), join inbox, community mailbox,
//! registry spine, subscription watches, gossip mesh, propagation verification.

use std::collections::HashMap;

use rekindle_types::dht_types::{
    CommunityMetadata, ChannelEntry, ChannelKind, EncryptedMekCopy,
    MekVaultEntry, MemberSummary, OnboardingConfig, WelcomeScreen,
    MANIFEST_METADATA, MANIFEST_CHANNELS, MANIFEST_ONBOARDING,
    MANIFEST_REGISTRY_SPINE, MANIFEST_WELCOME,
    CHANNEL_OWNER_SUBKEY_COUNT, CHANNEL_MEMBER_SUBKEY_COUNT,
    REGISTRY_OWNER_SUBKEY_COUNT, REGISTRY_MEMBER_SUBKEY_COUNT,
};
use rekindle_types::session_types::CommunityMembership;
use rekindle_types::transport::RecordSchema;

use crate::io::Confirm;
use crate::time::timestamp_ms;
use crate::ChatError;
use super::CommunityService;

use aws_lc_rs::rand::SecureRandom;

const SLOTS_PER_SEGMENT: usize = 255;
const CREATOR_SLOT: u32 = 0;

#[derive(Debug, Clone, serde::Serialize)]
pub struct CommunityCreated {
    pub governance_key: String,
    pub registry_key: String,
    pub community_name: String,
    pub community_mailbox_key: String,
    pub join_inbox_key: String,
}

impl CommunityService {
    pub async fn create_community(
        &self,
        name: &str,
        description: &str,
    ) -> Result<CommunityCreated, ChatError> {
        {
            let meta = self.session_meta.read();
            if let Some(existing) = meta.communities.values().find(|m| m.community_name == name) {
                return Ok(CommunityCreated {
                    governance_key: existing.governance_key.clone(),
                    registry_key: existing.registry_key.clone(),
                    community_name: name.to_string(),
                    community_mailbox_key: existing.community_mailbox_key.clone(),
                    join_inbox_key: existing.join_inbox_key.clone(),
                });
            }
        }

        let identity = {
            let meta = self.session_meta.read();
            meta.identity.clone().ok_or(ChatError::NotInitialized)?
        };

        let now = timestamp_ms();

        // ── Step 1: Generate shared slot_seed ──────────────────────────
        let mut slot_seed = [0u8; 32];
        aws_lc_rs::rand::SystemRandom::new()
            .fill(&mut slot_seed)
            .map_err(|e| ChatError::Internal(format!("slot_seed gen: {e}")))?;
        let slot_seed_hex = hex::encode(slot_seed);

        // ── Step 2: Derive 255 member slot public keys ─────────────────
        let mut member_pubkeys = Vec::with_capacity(SLOTS_PER_SEGMENT);
        for i in 0..SLOTS_PER_SEGMENT {
            let kp = rekindle_identity::derive_slot_keypair(&slot_seed, i as u32)
                .map_err(|e| ChatError::Internal(format!("slot keypair {i}: {e}")))?;
            member_pubkeys.push(kp.public_key_bytes());
        }

        // ── Step 3: Create governance (DFLT) + derive pseudonym ────────
        let (governance_record, governance_keypair) = self.io
            .create_record(RecordSchema::SingleWriter {
                subkey_count: rekindle_types::dht_types::MANIFEST_SUBKEY_COUNT,
            })
            .await?;
        let governance_key = governance_record.key().to_string();
        let gov_short = &governance_key[..12.min(governance_key.len())];

        // Store governance keypair early — write_signed_governance loads it from vault
        self.vault.store_key(
            &rekindle_storage::keys::labels::governance_keypair(gov_short),
            &governance_keypair,
        )?;

        let pseudonym_hex = self.io.pseudonym_hex(&governance_key)?;
        let pseudonym_seed = self.io.pseudonym_seed(&governance_key)?;
        let community_x25519_seed = rekindle_identity::x25519_seed_from(&pseudonym_seed);
        let our_x25519_dh_key = rekindle_identity::x25519_public_from_raw_seed(&community_x25519_seed)
            .map_err(|e| ChatError::Internal(format!("x25519: {e}")))?;
        let x25519_pub_hex = our_x25519_dh_key.to_hex();

        // Derive creator's slot keypair for SMPL writes
        let creator_slot_kp = rekindle_identity::derive_slot_keypair(&slot_seed, CREATOR_SLOT)
            .map_err(|e| ChatError::Internal(format!("creator slot keypair: {e}")))?;
        let creator_slot_kp_bytes = {
            let mut buf = [0u8; 64];
            buf[..32].copy_from_slice(&creator_slot_kp.public_key_bytes());
            let mut ikm = Vec::with_capacity(36);
            ikm.extend_from_slice(&slot_seed);
            ikm.extend_from_slice(&CREATOR_SLOT.to_le_bytes());
            let seed = blake3::derive_key(rekindle_identity::derivation_tags::SLOT_KEYPAIR, &ikm);
            buf[32..].copy_from_slice(&seed);
            buf
        };

        // ── Step 4: Create community mailbox ───────────────────────────
        let (community_mailbox_record, mailbox_keypair) = self.io
            .create_record(RecordSchema::SingleWriter { subkey_count: 1 })
            .await?;
        let community_mailbox_key = community_mailbox_record.key().to_string();
        let (_route_id, route_blob) = self.io.allocate_route().await?;
        self.io.write_record(
            &community_mailbox_record, 0, &route_blob,
            Some(&mailbox_keypair), Confirm::Accepted,
        ).await?;

        // ── Step 5: Create member registry (SMPL, o_cnt=0) ─────────────
        let (registry_record, _) = self.io
            .create_record(RecordSchema::MultiWriter {
                owner_subkeys: REGISTRY_OWNER_SUBKEY_COUNT,
                member_subkeys: REGISTRY_MEMBER_SUBKEY_COUNT,
                member_keys: member_pubkeys.clone(),
            })
            .await?;
        let registry_key = registry_record.key().to_string();

        // Write creator's signed MemberSummary to their slot (CREATOR_SLOT)
        let pseudonym_kp = rekindle_identity::SigningKeypair::from_seed(&pseudonym_seed)
            .map_err(|e| ChatError::Internal(format!("pseudonym keypair: {e}")))?;
        let mut owner_member = MemberSummary {
            pseudonym_key: pseudonym_hex.clone(),
            display_name: identity.display_name.clone(),
            role_ids: vec![0],
            joined_at: now,
            subkey_index: CREATOR_SLOT,
            onboarding_complete: true,
            timeout_until: None,
            x25519_pub: Some(x25519_pub_hex),
            profile_dht_key: Some(identity.profile_dht_key.clone()),
            channel_records: HashMap::new(),
            signature: Vec::new(),
        };
        let sig = pseudonym_kp.sign_raw(&owner_member.signing_bytes());
        owner_member.signature = sig.to_vec();
        let member_bytes = serde_json::to_vec(&owner_member)
            .map_err(|e| ChatError::Serialization(format!("member: {e}")))?;
        self.io.write_record(
            &registry_record, CREATOR_SLOT, &member_bytes,
            Some(&creator_slot_kp_bytes), Confirm::Accepted,
        ).await?;

        // ── Step 6: Create MEK vault (separate DFLT, operator-owned) ───
        let (mek_vault_record, mek_vault_keypair) = self.io
            .create_record(RecordSchema::SingleWriter { subkey_count: 1 })
            .await?;
        let mek_vault_key = mek_vault_record.key().to_string();
        let mek_vault_keypair_hex = hex::encode(&mek_vault_keypair);

        // ── Step 7: Create join inbox ──────────────────────────────────
        let (join_inbox_record, join_inbox_keypair) = self.io
            .create_record(RecordSchema::SingleWriter { subkey_count: 32 })
            .await?;
        let join_inbox_key = join_inbox_record.key().to_string();
        let join_inbox_keypair_hex = hex::encode(&join_inbox_keypair);
        self.io.write_record(
            &join_inbox_record, 0, b"[]",
            Some(&join_inbox_keypair), Confirm::Accepted,
        ).await?;

        // ── Step 8: Create shared SMPL channel record for #general ─────
        let general_id = uuid::Uuid::new_v4().to_string();
        let (channel_record, _) = self.io
            .create_record(RecordSchema::MultiWriter {
                owner_subkeys: CHANNEL_OWNER_SUBKEY_COUNT,
                member_subkeys: CHANNEL_MEMBER_SUBKEY_COUNT,
                member_keys: member_pubkeys,
            })
            .await?;

        let general = ChannelEntry {
            id: general_id.clone(),
            name: "general".to_string(),
            kind: ChannelKind::Text,
            sort_order: 0,
            category_id: None,
            topic: "General discussion".to_string(),
            slowmode_seconds: 0,
            nsfw: false,
            message_record_key: Some(channel_record.key().to_string()),
            mek_generation: 1,
            log_key: None,
            member_log_index_key: None,
        };
        let channels_bytes = serde_json::to_vec(&vec![general])
            .map_err(|e| ChatError::Serialization(format!("channels: {e}")))?;
        self.write_signed_governance(&governance_key, MANIFEST_CHANNELS, &channels_bytes).await?;

        let mut channel_record_keys = HashMap::new();
        let mut channel_name_to_id = HashMap::new();
        let mut channel_slowmode = HashMap::new();
        channel_record_keys.insert(general_id.clone(), channel_record.key().to_string());
        channel_name_to_id.insert("general".to_string(), general_id.clone());
        channel_slowmode.insert(general_id.clone(), 0);

        // ── Step 9: Generate MEK, wrap for creator, write to vault ──────
        let mut mek_key = [0u8; 32];
        aws_lc_rs::rand::SystemRandom::new()
            .fill(&mut mek_key)
            .map_err(|e| ChatError::Internal(format!("MEK gen: {e}")))?;

        let mek_wire = crate::crypto::mek::mek_to_wire(&mek_key, 1);
        let wrapped_mek = crate::crypto::mek::wrap_mek(
            &community_x25519_seed, &our_x25519_dh_key, &mek_wire,
        )?;

        // Wrap slot_seed for the creator via ECDH (same mechanism as MEK wrapping)
        let wrapped_creator_slot_seed = crate::crypto::mek::wrap_mek(
            &community_x25519_seed, &our_x25519_dh_key, &slot_seed,
        )?;

        let vault_entry = MekVaultEntry {
            channel_id: general_id.clone(),
            generation: 1,
            rotator_pseudonym: pseudonym_hex.clone(),
            rotator_x25519_pub: Some(our_x25519_dh_key),
            copies: vec![EncryptedMekCopy {
                target_pseudonym: pseudonym_hex.clone(),
                encrypted_mek: wrapped_mek,
            }],
            wrapped_slot_seeds: vec![rekindle_types::dht_types::WrappedSlotSeed {
                target_pseudonym: pseudonym_hex.clone(),
                encrypted_slot_seed: wrapped_creator_slot_seed,
            }],
        };
        let vault_bytes = serde_json::to_vec(&vec![vault_entry])
            .map_err(|e| ChatError::Serialization(format!("mek vault: {e}")))?;
        self.io.write_record(
            &mek_vault_record, 0, &vault_bytes,
            Some(&mek_vault_keypair), Confirm::Accepted,
        ).await?;
        self.mek_cache.insert(&governance_key, &general_id, mek_key, 1);

        // ── Step 10: Write governance metadata ─────────────────────────
        let metadata = CommunityMetadata {
            name: name.to_string(),
            description: Some(description.to_string()),
            icon_hash: None,
            banner_hash: None,
            created_at: now,
            owner_pseudonym: pseudonym_hex.clone(),
            last_refreshed: now,
            join_policy: rekindle_types::dht_types::JoinPolicy::AutoAllow,
            community_mailbox_key: community_mailbox_key.clone(),
            operator_pseudonyms: vec![pseudonym_hex.clone()],
            max_members: 255,
            mek_rotation_interval_hours: 168,
            join_inbox_key: join_inbox_key.clone(),
            join_inbox_keypair_hex: join_inbox_keypair_hex.clone(),
            slot_seed_hex: slot_seed_hex.clone(),
            mek_vault_key: mek_vault_key.clone(),
            mek_vault_keypair_hex: mek_vault_keypair_hex.clone(),
        };
        let metadata_bytes = serde_json::to_vec(&metadata)
            .map_err(|e| ChatError::Serialization(format!("metadata: {e}")))?;

        let spine = serde_json::json!({ "primary_key": registry_key, "segments": [] });
        let spine_bytes = serde_json::to_vec(&spine)
            .map_err(|e| ChatError::Serialization(format!("spine: {e}")))?;
        self.write_signed_governance(&governance_key, MANIFEST_REGISTRY_SPINE, &spine_bytes).await?;
        self.write_signed_governance(&governance_key, MANIFEST_METADATA, &metadata_bytes).await
            .map_err(|e| ChatError::Internal(format!(
                "governance metadata write FAILED for community '{name}': {e}"
            )))?;

        let onboarding = OnboardingConfig { enabled: false, steps: Vec::new(), require_rules_agreement: false };
        self.write_signed_governance(
            &governance_key, MANIFEST_ONBOARDING,
            &serde_json::to_vec(&onboarding).unwrap_or_default(),
        ).await?;

        let welcome = WelcomeScreen { title: format!("Welcome to {name}!"), body: description.to_string(), rules: Vec::new() };
        self.write_signed_governance(
            &governance_key, MANIFEST_WELCOME,
            &serde_json::to_vec(&welcome).unwrap_or_default(),
        ).await?;

        // ── Step 12: Update session meta ──────────────────────────────
        {
            let mut meta = self.session_meta.write();
            meta.communities.insert(governance_key.clone(), CommunityMembership {
                community_name: name.to_string(),
                governance_key: governance_key.clone(),
                registry_key: registry_key.clone(),
                pseudonym_key: pseudonym_hex,
                display_name: identity.display_name.clone(),
                role_ids: Vec::new(),
                slot_index: CREATOR_SLOT,
                channel_record_keys,
                channel_name_to_id,
                channel_slowmode,
                community_mailbox_key: community_mailbox_key.clone(),
                join_inbox_key: join_inbox_key.clone(),
                is_operator: true,
                locked_down: false,
                joined_at: now,
                last_seen_seqs: HashMap::new(),
                slot_seed: Some(slot_seed_hex),
                lamport_counter: 0,
            });
        }

        // ── Step 13: Watches + gossip mesh ────────────────────────────
        self.setup_community_watches(
            &governance_key, &registry_key, &join_inbox_key,
        ).await;

        if let Err(e) = self.io.join_mesh(&governance_key).await {
            tracing::warn!(community = name, error = %e, "gossip mesh join failed");
        }

        tracing::info!(
            name,
            governance = gov_short,
            registry = &registry_key[..12.min(registry_key.len())],
            channel = &channel_record.key()[..12.min(channel_record.key().len())],
            mek_vault = &mek_vault_key[..12.min(mek_vault_key.len())],
            creator_slot = CREATOR_SLOT,
            "community created"
        );

        Ok(CommunityCreated {
            governance_key,
            registry_key,
            community_name: name.to_string(),
            community_mailbox_key,
            join_inbox_key,
        })
    }
}
