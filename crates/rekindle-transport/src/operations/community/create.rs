//! Community creation — governance, registry, mailbox, inbox, default channel, MEK.

use std::sync::Arc;

use parking_lot::RwLock;
use tracing::info;

use crate::crypto::mek::{Mek, MekCache};
use crate::error::{TransportError, Result};
use crate::broadcast::node::TransportNode;
use crate::payload::dht_types::{
    ChannelEntry, ChannelKind, CommunityMetadata, JoinPolicy, MemberSummary,
};
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

pub async fn create_community(
    node: &TransportNode, session: &Session, name: &str,
    description: Option<&str>, mek_cache: &Arc<RwLock<MekCache>>,
    signing_key_bytes: &[u8; 32],
) -> Result<CommunityCreated> {
    info!(name, "creating community");
    let dht = node.dht()?;

    // Step 1: Create governance manifest
    let initial_metadata = CommunityMetadata {
        name: name.to_string(), description: description.map(String::from),
        icon_hash: None, banner_hash: None,
        created_at: rekindle_utils::timestamp_ms(),
        owner_pseudonym: session.identity.public_key_hex.clone(),
        last_refreshed: rekindle_utils::timestamp_ms(),
        join_policy: JoinPolicy::AutoAllow,
        community_mailbox_key: String::new(),
        operator_pseudonyms: vec![session.identity.public_key_hex.clone()],
        max_members: crate::payload::dht_types::REGISTRY_MAX_MEMBERS,
        mek_rotation_interval_hours: 168,
        join_inbox_key: String::new(), join_inbox_keypair_hex: String::new(),
    };
    let (governance_key, governance_keypair) = dht.governance().create(&initial_metadata).await
        .map_err(|e| TransportError::CommunityCreationFailed { reason: format!("governance manifest: {e}") })?;
    let governance_keypair_bytes = governance_keypair.map(|kp| super::super::identity::serialize_keypair(&kp)).unwrap_or_default();
    info!(key = %governance_key, "governance manifest created");

    // Step 2: Create community mailbox
    let gov_kp = crate::broadcast::node::deserialize_keypair(&governance_keypair_bytes)?;
    let community_mailbox_key = dht.mailbox().create_community_mailbox(gov_kp.clone()).await
        .map_err(|e| TransportError::CommunityCreationFailed { reason: format!("community mailbox: {e}") })?;
    info!(key = %community_mailbox_key, "community mailbox created");

    // Step 3: Allocate community route, publish to mailbox
    let (route_id, route_blob) = crate::broadcast::route::allocate_community(node).await
        .map_err(|e| TransportError::CommunityCreationFailed { reason: format!("community route: {e}") })?;
    dht.mailbox().update_community_route(&community_mailbox_key, &route_blob).await
        .map_err(|e| TransportError::CommunityCreationFailed { reason: format!("mailbox route publish: {e}") })?;
    info!(route = route_id, "community route published to mailbox");

    // Step 4: Create member registry
    let (registry_key, registry_keypair) = dht.registry().create().await
        .map_err(|e| TransportError::CommunityCreationFailed { reason: format!("member registry: {e}") })?;
    let registry_keypair_bytes = registry_keypair.map(|kp| super::super::identity::serialize_keypair(&kp)).unwrap_or_default();
    info!(key = %registry_key, "member registry created");

    // Step 5: Create join inbox (DFLT with published keypair)
    #[allow(clippy::cast_possible_truncation)]
    let (inbox_key, inbox_keypair) = crate::broadcast::dht_writes::create_dflt(
        node, crate::payload::dht_types::JOIN_INBOX_SUBKEY_COUNT as u16, None,
    ).await
        .map_err(|e| TransportError::CommunityCreationFailed { reason: format!("join inbox: {e}") })?;
    let inbox_keypair_hex = inbox_keypair.map(|kp| hex::encode(super::super::identity::serialize_keypair(&kp))).unwrap_or_default();
    crate::broadcast::dht_writes::set(node, &inbox_key, 0, b"[]".to_vec(), None).await
        .map_err(|e| TransportError::CommunityCreationFailed { reason: format!("inbox seed: {e}") })?;
    let inbox_subkeys: Vec<u32> = (0..crate::payload::dht_types::JOIN_INBOX_SUBKEY_COUNT).collect();
    let _ = crate::broadcast::dht_writes::watch(node, &inbox_key, &inbox_subkeys).await;
    info!(key = %inbox_key, "join inbox created, seeded, and watched");

    // Step 6: Create default #general channel
    let channel_id = uuid::Uuid::new_v4().to_string();
    let channel_entry = ChannelEntry {
        id: channel_id.clone(), name: "general".to_string(), kind: ChannelKind::Text,
        sort_order: 0, category_id: None, topic: "General discussion".to_string(),
        slowmode_seconds: 0, nsfw: false, message_record_key: None, mek_generation: 1, log_key: None,
    };
    dht.governance().write_channels(&governance_key, &[channel_entry]).await
        .map_err(|e| TransportError::CommunityCreationFailed { reason: format!("channel directory: {e}") })?;

    // Step 7: Generate MEK, wrap to creator, publish to vault
    let mek = Mek::generate(1);
    let mek_wire = mek.to_wire_bytes();
    mek_cache.write().insert(&governance_key, &channel_id, mek);
    let creator_pseudonym = crate::crypto::pseudonym::derive_community_pseudonym(signing_key_bytes, &governance_key);
    let creator_pseudonym_pub = creator_pseudonym.verifying_key().to_bytes();
    let creator_pseudonym_hex = hex::encode(creator_pseudonym_pub);
    let wrapped_mek = crate::crypto::mek::wrap_mek(&creator_pseudonym, &creator_pseudonym_pub, &mek_wire)
        .map_err(|e| TransportError::CommunityCreationFailed { reason: format!("mek wrap: {e}") })?;
    let vault_entry = crate::payload::dht_types::MekVaultEntry {
        channel_id: channel_id.clone(), generation: 1,
        rotator_pseudonym: creator_pseudonym_hex.clone(),
        copies: vec![crate::payload::dht_types::EncryptedMekCopy {
            target_pseudonym: creator_pseudonym_hex, encrypted_mek: wrapped_mek,
        }],
    };
    dht.registry().write_mek_vault(&registry_key, &[vault_entry]).await
        .map_err(|e| TransportError::CommunityCreationFailed { reason: format!("mek vault: {e}") })?;
    info!("initial MEK published to vault");

    // Step 8: Register creator as owner
    let our_pseudonym_key = session.identity.public_key_hex.clone();
    let owner_member = MemberSummary {
        pseudonym_key: our_pseudonym_key.clone(), display_name: session.identity.display_name.clone(),
        role_ids: vec![0], joined_at: rekindle_utils::timestamp_ms(), subkey_index: 0,
        onboarding_complete: true, timeout_until: None,
        profile_dht_key: Some(session.identity.profile_dht_key.clone()),
        channel_records: std::collections::HashMap::new(),
    };
    dht.registry().write_member_index(&registry_key, &[owner_member]).await
        .map_err(|e| TransportError::CommunityCreationFailed { reason: format!("owner registration: {e}") })?;

    // Step 9: Write final metadata with inbox key
    let final_metadata = CommunityMetadata {
        community_mailbox_key: community_mailbox_key.clone(),
        join_inbox_key: inbox_key.clone(), join_inbox_keypair_hex: inbox_keypair_hex,
        ..initial_metadata
    };
    dht.governance().write_metadata(&governance_key, &final_metadata).await
        .map_err(|e| TransportError::CommunityCreationFailed { reason: format!("metadata update: {e}") })?;

    // Step 10: Write registry spine
    let spine = serde_json::json!({ "primary_key": registry_key, "segments": [] });
    let spine_bytes = serde_json::to_vec(&spine).map_err(|e| TransportError::SerializationFailed { reason: e.to_string() })?;
    crate::broadcast::dht_writes::set(node, &governance_key, crate::payload::dht_types::MANIFEST_REGISTRY_SPINE, spine_bytes, None).await
        .map_err(|e| TransportError::CommunityCreationFailed { reason: format!("registry spine: {e}") })?;

    info!(governance = %governance_key, registry = %registry_key, mailbox = %community_mailbox_key, inbox = %inbox_key, "community created — verifying network propagation");

    // Step 11: Verify critical records propagated
    let verify_deadline = std::time::Duration::from_secs(30);
    let verify_start = std::time::Instant::now();
    let mut backoff = std::time::Duration::from_millis(500);
    let ceiling = std::time::Duration::from_secs(5);

    loop {
        let metadata_ok = match crate::broadcast::dht_writes::get(
            node, &governance_key, crate::payload::dht_types::MANIFEST_METADATA, true,
        ).await {
            Ok(Some(data)) => {
                serde_json::from_slice::<CommunityMetadata>(&data)
                    .map(|m| !m.join_inbox_key.is_empty())
                    .unwrap_or(false)
            }
            _ => false,
        };

        if metadata_ok {
            info!(governance = %governance_key, elapsed_ms = verify_start.elapsed().as_millis(), "community verified — metadata propagated");
            break;
        }

        if verify_start.elapsed() >= verify_deadline {
            tracing::warn!(governance = %governance_key, "community created but metadata propagation not confirmed within 30s");
            break;
        }

        info!(governance = %governance_key, elapsed_secs = verify_start.elapsed().as_secs(), backoff_ms = backoff.as_millis(), "verify: not yet propagated, retrying");
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(ceiling);
    }

    Ok(CommunityCreated {
        governance_key, governance_keypair_bytes, registry_key, registry_keypair_bytes,
        community_mailbox_key, join_inbox_key: inbox_key,
        default_channel_id: channel_id,
        our_pseudonym_key, our_slot_index: 0, mek_generation: 1,
    })
}
