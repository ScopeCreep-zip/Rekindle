//! Community creation — governance record, registry, channels, MEK with
//! ECDH wrapping, join inbox, community mailbox with route, registry spine,
//! subscription watches, gossip mesh, propagation verification.

use std::collections::HashMap;

use rekindle_types::dht_types::{
    CommunityMetadata, ChannelEntry, ChannelKind, EncryptedMekCopy,
    MekVaultEntry, MemberSummary,
    MANIFEST_METADATA, MANIFEST_CHANNELS, MANIFEST_REGISTRY_SPINE,
    REGISTRY_MEMBER_INDEX, REGISTRY_MEK_VAULT,
};
use rekindle_types::session_types::CommunityMembership;
use rekindle_types::transport::RecordSchema;

use crate::io::Confirm;
use crate::time::timestamp_ms;
use crate::ChatError;
use super::CommunityService;

use aws_lc_rs::rand::SecureRandom;

#[derive(Debug, Clone, serde::Serialize)]
pub struct CommunityCreated {
    pub governance_key: String,
    pub registry_key: String,
    pub community_name: String,
    pub community_mailbox_key: String,
    pub join_inbox_key: String,
}

impl CommunityService {
    /// Create a new community.
    ///
    /// 11 steps: governance manifest → community mailbox with route →
    /// member registry → join inbox → default channel → MEK generation +
    /// ECDH wrapping → owner registration → registry spine → propagation
    /// verification → subscription watches → gossip mesh.
    pub async fn create_community(
        &self,
        name: &str,
        description: &str,
    ) -> Result<CommunityCreated, ChatError> {
        {
            let meta = self.session_meta.read();
            if meta.communities.values().any(|m| m.community_name == name) {
                return Err(ChatError::Internal(format!("community '{name}' already exists")));
            }
        }

        let identity = {
            let meta = self.session_meta.read();
            meta.identity.clone().ok_or(ChatError::NotInitialized)?
        };

        let now = timestamp_ms();

        // Step 1: Create governance manifest (DFLT, social + governance subkeys)
        let (governance_key, governance_keypair) = self.io
            .create_record(RecordSchema::SingleWriter {
                subkey_count: rekindle_types::dht_types::MANIFEST_SUBKEY_COUNT,
            })
            .await?;
        let gov_short = &governance_key[..12.min(governance_key.len())];

        // Derive community pseudonym from the actual governance key
        let pseudonym_hex = self.io.pseudonym_hex(&governance_key)?;
        let pseudonym_seed = self.io.pseudonym_seed(&governance_key)?;

        // Derive X25519 pub for MEK wrapping
        let community_x25519_seed = blake3::derive_key("rekindle identity x25519 v1", &pseudonym_seed);
        let x25519_key = rekindle_ratchet::crypto::dh::reusable_from_seed(&community_x25519_seed)
            .map_err(|e| ChatError::Internal(format!("x25519: {e}")))?;
        let x25519_pub_raw = x25519_key.compute_public_key()
            .map_err(|_| ChatError::Internal("x25519 pub derive".into()))?;
        let x25519_pub_hex = hex::encode(x25519_pub_raw.as_ref());

        // Step 2: Create community mailbox (DFLT 1 subkey) + allocate route
        let (community_mailbox_key, mailbox_keypair) = self.io
            .create_record(RecordSchema::SingleWriter { subkey_count: 1 })
            .await?;
        let (_route_id, route_blob) = self.io.allocate_route().await?;
        self.io.write_record(
            &community_mailbox_key, 0, &route_blob,
            Some(&mailbox_keypair), Confirm::Accepted,
        ).await?;

        // Step 3: Create member registry (SMPL: 11 owner + 245 member slots)
        let (registry_key, registry_keypair) = self.io
            .create_record(RecordSchema::MultiWriter {
                owner_subkeys: 11,
                member_subkeys: 1,
                member_count: 245,
            })
            .await?;

        // Step 4: Create join inbox (DFLT 32 subkeys)
        let (join_inbox_key, join_inbox_keypair) = self.io
            .create_record(RecordSchema::SingleWriter { subkey_count: 32 })
            .await?;
        let join_inbox_keypair_hex = hex::encode(&join_inbox_keypair);

        // Seed inbox subkey 0
        self.io.write_record(
            &join_inbox_key, 0, b"[]",
            Some(&join_inbox_keypair), Confirm::Accepted,
        ).await?;

        // Step 5: Write default #general channel
        let general_id = uuid::Uuid::new_v4().to_string();
        let general = ChannelEntry {
            id: general_id.clone(),
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
        let channels_bytes = serde_json::to_vec(&vec![general])
            .map_err(|e| ChatError::Serialization(format!("channels: {e}")))?;
        self.io.write_record(
            &governance_key, MANIFEST_CHANNELS, &channels_bytes,
            Some(&governance_keypair), Confirm::Accepted,
        ).await?;

        // Step 6: Generate MEK, ECDH-wrap for creator, write to vault
        let mut mek_key = [0u8; 32];
        aws_lc_rs::rand::SystemRandom::new()
            .fill(&mut mek_key)
            .map_err(|e| ChatError::Internal(format!("MEK gen: {e}")))?;

        let mek_wire = crate::crypto::mek::mek_to_wire(&mek_key, 1);
        let mut x25519_pub_bytes = [0u8; 32];
        x25519_pub_bytes.copy_from_slice(x25519_pub_raw.as_ref());

        let wrapped_mek = crate::crypto::mek::wrap_mek(
            &community_x25519_seed, &x25519_pub_bytes, &mek_wire,
        )?;

        let vault_entry = MekVaultEntry {
            channel_id: general_id.clone(),
            generation: 1,
            rotator_pseudonym: pseudonym_hex.clone(),
            copies: vec![EncryptedMekCopy {
                target_pseudonym: pseudonym_hex.clone(),
                encrypted_mek: wrapped_mek,
            }],
        };
        let vault_bytes = serde_json::to_vec(&vec![vault_entry])
            .map_err(|e| ChatError::Serialization(format!("mek vault: {e}")))?;
        self.io.write_record(
            &registry_key, REGISTRY_MEK_VAULT, &vault_bytes,
            Some(&registry_keypair), Confirm::Accepted,
        ).await?;

        self.mek_cache.insert(&governance_key, &general_id, mek_key, 1);

        // Step 7: Register creator as owner member with x25519_pub
        let owner_member = MemberSummary {
            pseudonym_key: pseudonym_hex.clone(),
            display_name: identity.display_name.clone(),
            role_ids: vec![0],
            joined_at: now,
            subkey_index: 11,
            onboarding_complete: true,
            timeout_until: None,
            x25519_pub: Some(x25519_pub_hex),
            profile_dht_key: Some(identity.profile_dht_key.clone()),
            channel_records: HashMap::new(),
        };
        let members_bytes = serde_json::to_vec(&vec![owner_member])
            .map_err(|e| ChatError::Serialization(format!("members: {e}")))?;
        self.io.write_record(
            &registry_key, REGISTRY_MEMBER_INDEX, &members_bytes,
            Some(&registry_keypair), Confirm::Accepted,
        ).await?;

        // Step 8: Write governance metadata (includes inbox key + mailbox key)
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
            max_members: 245,
            mek_rotation_interval_hours: 168,
            join_inbox_key: join_inbox_key.clone(),
            join_inbox_keypair_hex: join_inbox_keypair_hex.clone(),
        };
        let metadata_bytes = serde_json::to_vec(&metadata)
            .map_err(|e| ChatError::Serialization(format!("metadata: {e}")))?;

        // Step 9: Write registry spine (so joiners can discover the registry key)
        let spine = serde_json::json!({ "primary_key": registry_key, "segments": [] });
        let spine_bytes = serde_json::to_vec(&spine)
            .map_err(|e| ChatError::Serialization(format!("spine: {e}")))?;
        self.io.write_record(
            &governance_key, MANIFEST_REGISTRY_SPINE, &spine_bytes,
            Some(&governance_keypair), Confirm::Accepted,
        ).await?;

        // Step 10: Write metadata with Confirm::Propagated — this is the critical
        // record that must be discoverable before anyone can join. Metadata includes
        // the inbox key, so joiners can't submit requests until this propagates.
        self.io.write_record(
            &governance_key, MANIFEST_METADATA, &metadata_bytes,
            Some(&governance_keypair), Confirm::Propagated,
        ).await.map_err(|e| ChatError::Internal(format!(
            "governance metadata propagation FAILED for community '{name}': {e} — \
             the community exists but is not yet discoverable. Retry or wait \
             for DHT propagation."
        )))?;

        // Store keypairs in vault
        self.vault.store_key(
            &rekindle_storage::keys::labels::governance_keypair(gov_short),
            &governance_keypair,
        )?;
        let reg_short = &registry_key[..12.min(registry_key.len())];
        self.vault.store_key(
            &rekindle_storage::keys::labels::registry_keypair(reg_short),
            &registry_keypair,
        )?;

        // Update session meta
        {
            let mut meta = self.session_meta.write();
            meta.communities.insert(governance_key.clone(), CommunityMembership {
                community_name: name.to_string(),
                governance_key: governance_key.clone(),
                registry_key: registry_key.clone(),
                pseudonym_key: pseudonym_hex,
                display_name: identity.display_name.clone(),
                role_ids: Vec::new(),
                slot_index: 11,
                channel_record_keys: HashMap::new(),
                community_mailbox_key: community_mailbox_key.clone(),
                join_inbox_key: join_inbox_key.clone(),
                is_operator: true,
                locked_down: false,
                joined_at: now,
            });
        }

        // Step 11: Establish subscription watches
        self.setup_community_watches(
            &governance_key, &registry_key, &join_inbox_key,
        ).await;

        // Join gossip mesh
        if let Err(e) = self.io.join_mesh(&governance_key).await {
            tracing::warn!(
                community = name,
                error = %e,
                "gossip mesh join failed — real-time events will use watch+poll only"
            );
        }

        tracing::info!(
            name,
            governance = gov_short,
            registry = &registry_key[..12.min(registry_key.len())],
            "community created — metadata propagated, watches + gossip mesh active"
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