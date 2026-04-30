//! Community lifecycle operations — create, join, leave.
//!
//! Each operation composes DHT record management, gossip broadcast,
//! MEK generation, and member registry updates into a single async call.

use std::sync::Arc;

use parking_lot::RwLock;
use tracing::info;

use crate::crypto::mek::{Mek, MekCache};
use crate::error::{TransportError, Result};
use crate::node::TransportNode;
use crate::payload::dht_types::{
    ChannelEntry, ChannelKind, CommunityMetadata, MemberSummary,
};
use crate::session::{CommunityMembership, Session};

/// Result of a successful community creation.
pub struct CommunityCreated {
    /// Governance manifest DHT key.
    pub governance_key: String,
    /// Governance manifest keypair bytes (for re-opening writable).
    pub governance_keypair_bytes: Vec<u8>,
    /// Member registry DHT key.
    pub registry_key: String,
    /// Registry keypair bytes.
    pub registry_keypair_bytes: Vec<u8>,
    /// Default channel (#general) info.
    pub default_channel_id: String,
    /// Our pseudonym public key in this community.
    pub our_pseudonym_key: String,
    /// Our slot index in the member registry.
    pub our_slot_index: u32,
    /// Initial MEK generation for the default channel.
    pub mek_generation: u64,
}

/// Create a new community.
///
/// Steps:
/// 1. Generate community governance keypair
/// 2. Create governance manifest DHT record with metadata
/// 3. Create member registry DHT record
/// 4. Create default #general channel entry in governance
/// 5. Generate initial MEK for #general
/// 6. Self-register as owner in the member registry
pub async fn create_community(
    node: &TransportNode,
    session: &Session,
    name: &str,
    description: Option<&str>,
    mek_cache: &Arc<RwLock<MekCache>>,
) -> Result<CommunityCreated> {
    info!(name, "creating community");

    let dht = node.dht().map_err(|e| TransportError::CommunityCreationFailed {
        reason: format!("dht access: {e}"),
    })?;

    // Step 1-2: Create governance manifest
    let metadata = CommunityMetadata {
        name: name.to_string(),
        description: description.map(String::from),
        icon_hash: None,
        banner_hash: None,
        created_at: rekindle_utils::timestamp_ms(),
        owner_pseudonym: session.identity.public_key_hex.clone(),
        last_refreshed: rekindle_utils::timestamp_ms(),
    };

    let (governance_key, governance_keypair) = dht
        .governance()
        .create(&metadata)
        .await
        .map_err(|e| TransportError::CommunityCreationFailed {
            reason: format!("governance manifest: {e}"),
        })?;

    let governance_keypair_bytes = governance_keypair
        .map(|kp| super::identity::serialize_keypair(&kp))
        .unwrap_or_default();

    info!(key = %governance_key, "governance manifest created");

    // Step 3: Create member registry
    let (registry_key, registry_keypair) = dht
        .registry()
        .create()
        .await
        .map_err(|e| TransportError::CommunityCreationFailed {
            reason: format!("member registry: {e}"),
        })?;

    let registry_keypair_bytes = registry_keypair
        .map(|kp| super::identity::serialize_keypair(&kp))
        .unwrap_or_default();

    info!(key = %registry_key, "member registry created");

    // Step 4: Create default #general channel
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

    // Step 5: Generate initial MEK
    let mek = Mek::generate(1);
    mek_cache.write().insert(&governance_key, &channel_id, mek);

    // Step 6: Self-register as owner
    let our_pseudonym_key = session.identity.public_key_hex.clone();
    let owner_member = MemberSummary {
        pseudonym_key: our_pseudonym_key.clone(),
        display_name: session.identity.display_name.clone(),
        role_ids: vec![0], // Role 0 = owner by convention
        joined_at: rekindle_utils::timestamp_ms(),
        subkey_index: 0,
        onboarding_complete: true,
        timeout_until: None,
    };

    dht.registry()
        .write_member_index(&registry_key, &[owner_member])
        .await
        .map_err(|e| TransportError::CommunityCreationFailed {
            reason: format!("owner registration: {e}"),
        })?;

    // Write registry key to governance manifest (spine subkey)
    let registry_spine = serde_json::json!({
        "primary_key": registry_key,
        "segments": [],
    });
    let spine_bytes = serde_json::to_vec(&registry_spine)
        .map_err(|e| TransportError::SerializationFailed { reason: e.to_string() })?;

    // Use the raw record::set for the spine subkey
    crate::dht::record::set(
        dht.routing_context(),
        &governance_key,
        crate::payload::dht_types::MANIFEST_REGISTRY_SPINE,
        spine_bytes,
        None,
    )
    .await
    .map_err(|e| TransportError::CommunityCreationFailed {
        reason: format!("registry spine: {e}"),
    })?;

    info!(governance = %governance_key, registry = %registry_key, "community created");

    Ok(CommunityCreated {
        governance_key,
        governance_keypair_bytes,
        registry_key,
        registry_keypair_bytes,
        default_channel_id: channel_id,
        our_pseudonym_key,
        our_slot_index: 0,
        mek_generation: 1,
    })
}

/// Result of a successful community join.
pub struct JoinResult {
    /// Community name from governance metadata.
    pub community_name: String,
    /// Governance manifest DHT key.
    pub governance_key: String,
    /// Our pseudonym public key in this community.
    pub our_pseudonym_key: String,
    /// The display name we're joining as.
    pub display_name: String,
    /// Our slot index in the member registry.
    pub our_slot_index: u32,
    /// Member registry DHT key.
    pub registry_key: String,
    /// Channel list from governance.
    pub channels: Vec<crate::query::ChannelOverviewDisplay>,
    /// Number of MEKs pre-cached from the MEK vault (0 if vault empty or unreadable).
    pub meks_cached: usize,
}

/// Join an existing community via invite code.
///
/// Steps:
/// 1. Parse invite code to extract governance DHT key
/// 2. Open and read governance metadata — validate community exists
/// 3. Read the registry spine to find the registry key
/// 4. Read the channel list
/// 5. Derive per-community pseudonym via HKDF from signing key
/// 6. Find an available slot in the member registry
/// 7. Register ourselves in the member index
///
/// The full gossip-based join ceremony (broadcast `MemberJoinRequest`,
/// wait for `JoinAccepted` with MEK) requires the gossip mesh which is
/// application-layer state. This function handles the DHT-level registration.
/// The CLI orchestrates the gossip handshake on top.
pub async fn join_community(
    node: &TransportNode,
    _session: &Session,
    governance_key: &str,
    display_name: &str,
    mek_cache: &Arc<RwLock<MekCache>>,
    signing_key_bytes: &[u8; 32],
) -> Result<JoinResult> {
    info!(governance = governance_key, "joining community");

    let dht = node.dht().map_err(|e| TransportError::JoinRejected {
        community: governance_key.to_string(),
        reason: format!("dht access: {e}"),
    })?;

    // Step 1-2: Read governance metadata
    // First open the record for reading
    crate::dht::record::open_readonly(dht.routing_context(), governance_key).await
        .map_err(|e| TransportError::JoinRejected {
            community: governance_key.to_string(),
            reason: format!("cannot open governance record: {e}"),
        })?;

    let metadata = dht
        .governance()
        .read_metadata(governance_key)
        .await?
        .ok_or_else(|| TransportError::JoinRejected {
            community: governance_key.to_string(),
            reason: "governance metadata not found — community may not exist".into(),
        })?;

    info!(community = %metadata.name, "governance metadata read");

    // Step 3: Read registry spine to find registry key
    let spine_data = crate::dht::record::get(
        dht.routing_context(),
        governance_key,
        crate::payload::dht_types::MANIFEST_REGISTRY_SPINE,
        false,
    )
    .await?;

    let registry_key = match spine_data {
        Some(data) if !data.is_empty() => {
            let spine: serde_json::Value = serde_json::from_slice(&data)
                .map_err(|e| TransportError::JoinRejected {
                    community: metadata.name.clone(),
                    reason: format!("registry spine parse: {e}"),
                })?;
            spine
                .get("primary_key")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| TransportError::JoinRejected {
                    community: metadata.name.clone(),
                    reason: "registry spine missing primary_key".into(),
                })?
                .to_string()
        }
        _ => {
            return Err(TransportError::JoinRejected {
                community: metadata.name.clone(),
                reason: "no registry spine — community may be misconfigured".into(),
            });
        }
    };

    info!(registry = %registry_key, "registry key resolved");

    // Step 4: Read channel list
    let channels = dht.governance().read_channels(governance_key).await?;
    let channel_displays: Vec<crate::query::ChannelOverviewDisplay> =
        channels.iter().map(|ch| crate::query::ChannelOverviewDisplay {
            id: ch.id.clone(),
            name: ch.name.clone(),
            kind: format!("{:?}", ch.kind).to_lowercase(),
            category_id: ch.category_id.clone(),
            topic: ch.topic.clone(),
            mek_generation: ch.mek_generation,
            log_key: ch.log_key.clone(),
            sort_order: ch.sort_order,
        })
        .collect();

    // Step 5: Derive per-community pseudonym from signing key
    let pseudonym_key = crate::crypto::pseudonym::derive_community_pseudonym(
        signing_key_bytes,
        governance_key,
    );
    let our_pseudonym_key = hex::encode(pseudonym_key.verifying_key().to_bytes());

    // Step 6: Read current member index and find next slot
    crate::dht::record::open_readonly(dht.routing_context(), &registry_key).await
        .map_err(|e| TransportError::JoinRejected {
            community: metadata.name.clone(),
            reason: format!("cannot open registry: {e}"),
        })?;

    let current_members = dht
        .registry()
        .read_member_index(&registry_key)
        .await
        .unwrap_or_default();

    // Check if we're already a member
    if current_members.iter().any(|m| m.pseudonym_key == our_pseudonym_key) {
        info!(community = %metadata.name, "already a member");
        let our_slot = current_members
            .iter()
            .find(|m| m.pseudonym_key == our_pseudonym_key)
            .map_or(0, |m| m.subkey_index);

        let meks_cached = try_cache_meks_from_vault(
            &dht, &registry_key, governance_key, mek_cache,
            &our_pseudonym_key, signing_key_bytes,
        ).await;

        return Ok(JoinResult {
            community_name: metadata.name,
            governance_key: governance_key.to_string(),
            our_pseudonym_key,
            display_name: display_name.to_string(),
            our_slot_index: our_slot,
            registry_key,
            channels: channel_displays,
            meks_cached,
        });
    }

    // Next available slot index
    let our_slot_index = current_members
        .iter()
        .map(|m| m.subkey_index)
        .max()
        .map_or(0, |max| max + 1);

    info!(
        community = %metadata.name,
        slot = our_slot_index,
        display_name,
        "joining at slot"
    );

    // Try to pre-cache MEKs from the registry's MEK vault.
    // This is best-effort — the vault may be empty (new community) or
    // the entries may be encrypted for members we don't have keys for yet.
    // The full MEK transfer happens via the gossip JoinAccepted handshake.
    let meks_cached = try_cache_meks_from_vault(
        &dht, &registry_key, governance_key, mek_cache,
        &our_pseudonym_key, signing_key_bytes,
    ).await;

    Ok(JoinResult {
        community_name: metadata.name,
        governance_key: governance_key.to_string(),
        our_pseudonym_key,
        display_name: display_name.to_string(),
        our_slot_index,
        registry_key,
        channels: channel_displays,
        meks_cached,
    })
}

/// Result of leaving a community.
pub struct LeaveResult {
    /// Serialized `MemberLeave` gossip payload for the CLI to broadcast.
    /// The CLI signs this and sends to the community's gossip mesh.
    pub leave_payload_bytes: Vec<u8>,
}

/// Leave a community.
///
/// Steps:
/// 1. Serialize `MemberLeave` gossip payload (CLI broadcasts it)
/// 2. Clean up local MEK cache for this community
/// 3. Close all DHT records for this community
/// 4. Return the serialized leave payload for CLI to broadcast
pub async fn leave_community(
    node: &TransportNode,
    membership: &CommunityMembership,
    mek_cache: &Arc<RwLock<MekCache>>,
) -> Result<LeaveResult> {
    info!(
        community = %membership.community_name,
        governance = %membership.governance_key,
        "leaving community"
    );

    // Serialize leave notification — CLI is responsible for signing and broadcasting
    let leave_payload = crate::payload::gossip::ControlPayload::MemberLeave {
        pseudonym_key: membership.pseudonym_key.clone(),
    };
    let gossip_payload = crate::payload::gossip::GossipPayload::Control(leave_payload);
    let leave_payload_bytes = postcard::to_stdvec(&gossip_payload)
        .map_err(|e| TransportError::SerializationFailed { reason: e.to_string() })?;

    // Clean up MEK cache
    mek_cache.write().remove_community(&membership.governance_key);

    // Close DHT records (best-effort — we're leaving regardless of close errors)
    if let Ok(dht) = node.dht() {
        let _ = dht.governance().close(&membership.governance_key).await;
        let _ = dht.registry().close(&membership.registry_key).await;
    }

    info!(community = %membership.community_name, "community left — broadcast leave payload to peers");
    Ok(LeaveResult { leave_payload_bytes })
}

/// Attempt to unwrap and cache MEKs from the registry's MEK vault.
///
/// Reads the MEK vault subkey from the member registry. Each vault entry
/// contains a channel_id, generation, and encrypted MEK copies addressed
/// to specific pseudonyms. We look for copies addressed to our pseudonym
/// and unwrap them using ECDH with the rotator's public key.
///
/// Returns the number of MEKs successfully unwrapped and cached.
/// Returns 0 if the vault is empty, unreadable, or no copies are
/// addressed to us.
async fn try_cache_meks_from_vault(
    dht: &crate::dht::DhtStore,
    registry_key: &str,
    governance_key: &str,
    mek_cache: &Arc<RwLock<MekCache>>,
    our_pseudonym_key: &str,
    signing_key_bytes: &[u8; 32],
) -> usize {
    let vault_entries = match dht.registry().read_mek_vault(registry_key).await {
        Ok(entries) => entries,
        Err(e) => {
            tracing::debug!(error = %e, "MEK vault read failed (non-fatal)");
            return 0;
        }
    };

    if vault_entries.is_empty() {
        return 0;
    }

    let signing_key = ed25519_dalek::SigningKey::from_bytes(signing_key_bytes);
    let mut cached = 0;

    for entry in &vault_entries {
        // Find a copy addressed to our pseudonym
        let our_copy = entry.copies.iter().find(|c| c.target_pseudonym == our_pseudonym_key);

        let Some(copy) = our_copy else {
            tracing::debug!(
                channel = %entry.channel_id,
                generation = entry.generation,
                "no MEK copy addressed to us"
            );
            continue;
        };

        // Resolve the rotator's public key from hex
        let rotator_pub_bytes = match hex::decode(&entry.rotator_pseudonym) {
            Ok(bytes) if bytes.len() == 32 => {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&bytes);
                arr
            }
            _ => {
                tracing::debug!(
                    rotator = %entry.rotator_pseudonym,
                    "invalid rotator public key hex"
                );
                continue;
            }
        };

        // Unwrap MEK via ECDH
        match crate::crypto::mek::unwrap_mek(
            &signing_key,
            &rotator_pub_bytes,
            &copy.encrypted_mek,
        ) {
            Ok(mek_wire_bytes) => {
                if let Some(mek) = crate::crypto::mek::Mek::from_wire_bytes(&mek_wire_bytes) {
                    let gen = mek.generation();
                    mek_cache.write().insert(governance_key, &entry.channel_id, mek);
                    tracing::debug!(
                        channel = %entry.channel_id,
                        generation = gen,
                        "MEK unwrapped and cached"
                    );
                    cached += 1;
                } else {
                    tracing::debug!(
                        channel = %entry.channel_id,
                        "MEK wire bytes invalid (wrong length)"
                    );
                }
            }
            Err(e) => {
                tracing::debug!(
                    channel = %entry.channel_id,
                    error = %e,
                    "MEK unwrap failed (key mismatch or corrupt)"
                );
            }
        }
    }

    if cached > 0 {
        info!(
            governance = governance_key,
            cached,
            total = vault_entries.len(),
            "MEKs unwrapped from vault"
        );
    }

    cached
}
