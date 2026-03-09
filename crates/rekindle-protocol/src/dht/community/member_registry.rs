//! Member registry operations for SMPL multi-writer DHT records.
//!
//! The member registry is a SMPL schema record where:
//! - The coordinator (owner) controls subkeys 0-1:
//!   - Subkey 0: Member index (list of all members with subkey assignments)
//!   - Subkey 1: MEK vault (encrypted MEK copies for key distribution)
//! - Each member gets 1 subkey for their presence data.
//!
//! Member subkeys start at offset `REGISTRY_OWNER_SUBKEY_COUNT` (2),
//! so member at index 0 writes to subkey 2, index 1 to subkey 3, etc.

use crate::dht::DHTManager;
use crate::error::ProtocolError;

use super::types::{
    MEKVaultEntry, MemberPresence, MemberSummary, RegistrySegmentInfo, RegistrySpine,
    MANIFEST_REGISTRY_SPINE, REGISTRY_MEK_VAULT, REGISTRY_MEMBER_INDEX,
    REGISTRY_MEMBER_SUBKEY_COUNT, REGISTRY_OWNER_SUBKEY_COUNT,
};

/// Maximum member slots per registry segment.
///
/// Veilid's `DHTSchemaSMPL` has `MAX_MEMBER_COUNT = 256` and `MAX_WRITER_COUNT = 256`.
/// Since the owner counts as 1 writer, we can have at most 255 member writers
/// (1 owner + 255 members = 256 total writers).
pub const SLOTS_PER_SEGMENT: u32 = 255;

/// Create a new member registry SMPL record.
///
/// The coordinator owns `REGISTRY_OWNER_SUBKEY_COUNT` subkeys (member index + MEK vault).
/// Initially created with no members — members are added via [`add_member_to_registry`]
/// which requires rebuilding the SMPL schema.
///
/// Returns `(record_key, owner_keypair)`.
pub async fn create_member_registry(
    dht: &DHTManager,
) -> Result<(String, Option<veilid_core::KeyPair>), ProtocolError> {
    // Start with a DFLT record for the initial empty registry.
    // When the first member joins, this will be recreated as SMPL.
    // SMPL schema with 0 members is just DFLT with owner_subkey_count subkeys.
    let (key, owner_keypair) = dht
        .create_record(u32::from(REGISTRY_OWNER_SUBKEY_COUNT))
        .await?;

    // Initialize empty member index
    let empty_index: Vec<MemberSummary> = Vec::new();
    write_member_index(dht, &key, &empty_index).await?;

    // Initialize empty MEK vault
    let empty_vault: Vec<MEKVaultEntry> = Vec::new();
    write_mek_vault(dht, &key, &empty_vault).await?;

    tracing::info!(key = %key, "member registry record created");
    Ok((key, owner_keypair))
}

/// Create a SMPL member registry record with initial members.
///
/// Each member's `BareMemberId` must be pre-computed by the caller using
/// `api.crypto()?.get(CRYPTO_KIND_VLD0)?.generate_member_id(public_key)`.
///
/// Returns `(record_key, owner_keypair)`.
pub async fn create_member_registry_with_members(
    dht: &DHTManager,
    members: &[(veilid_core::BareMemberId, MemberSummary)],
) -> Result<(String, Option<veilid_core::KeyPair>), ProtocolError> {
    let smpl_members: Vec<veilid_core::DHTSchemaSMPLMember> = members
        .iter()
        .map(|(member_id, _)| veilid_core::DHTSchemaSMPLMember {
            m_key: member_id.clone(),
            m_cnt: REGISTRY_MEMBER_SUBKEY_COUNT,
        })
        .collect();

    let (key, owner_keypair) = dht
        .create_smpl_record(REGISTRY_OWNER_SUBKEY_COUNT, smpl_members)
        .await?;

    // Write initial member index
    let index: Vec<MemberSummary> = members.iter().map(|(_, s)| s.clone()).collect();
    write_member_index(dht, &key, &index).await?;

    // Initialize empty MEK vault
    let empty_vault: Vec<MEKVaultEntry> = Vec::new();
    write_mek_vault(dht, &key, &empty_vault).await?;

    tracing::info!(key = %key, member_count = members.len(), "SMPL member registry created");
    Ok((key, owner_keypair))
}

// ── Member index (owner subkey 0) ──

/// Read the member index from the registry.
pub async fn read_member_index(
    dht: &DHTManager,
    key: &str,
) -> Result<Vec<MemberSummary>, ProtocolError> {
    match dht.get_value(key, REGISTRY_MEMBER_INDEX).await? {
        Some(data) => serde_json::from_slice(&data)
            .map_err(|e| ProtocolError::Deserialization(format!("member index: {e}"))),
        None => Ok(Vec::new()),
    }
}

/// Write the member index to the registry (requires registry_owner_keypair).
pub async fn write_member_index(
    dht: &DHTManager,
    key: &str,
    members: &[MemberSummary],
) -> Result<(), ProtocolError> {
    let bytes = serde_json::to_vec(members)
        .map_err(|e| ProtocolError::Serialization(format!("member index: {e}")))?;
    dht.set_value(key, REGISTRY_MEMBER_INDEX, bytes).await
}

// ── MEK vault (owner subkey 1) ──

/// Read the MEK vault from the registry.
pub async fn read_mek_vault(
    dht: &DHTManager,
    key: &str,
) -> Result<Vec<MEKVaultEntry>, ProtocolError> {
    match dht.get_value(key, REGISTRY_MEK_VAULT).await? {
        Some(data) => serde_json::from_slice(&data)
            .map_err(|e| ProtocolError::Deserialization(format!("MEK vault: {e}"))),
        None => Ok(Vec::new()),
    }
}

/// Write the MEK vault to the registry (requires registry_owner_keypair).
pub async fn write_mek_vault(
    dht: &DHTManager,
    key: &str,
    vault: &[MEKVaultEntry],
) -> Result<(), ProtocolError> {
    let bytes = serde_json::to_vec(vault)
        .map_err(|e| ProtocolError::Serialization(format!("MEK vault: {e}")))?;
    dht.set_value(key, REGISTRY_MEK_VAULT, bytes).await
}

// ── Member presence (member subkeys, starting at REGISTRY_OWNER_SUBKEY_COUNT) ──

/// Calculate the DHT subkey index for a member given their index in the member list.
pub fn member_subkey(member_index: u32) -> u32 {
    u32::from(REGISTRY_OWNER_SUBKEY_COUNT) + member_index
}

/// Read a member's presence from their SMPL subkey.
pub async fn read_member_presence(
    dht: &DHTManager,
    key: &str,
    member_index: u32,
) -> Result<Option<MemberPresence>, ProtocolError> {
    let subkey = member_subkey(member_index);
    match dht.get_value(key, subkey).await? {
        Some(data) => {
            let presence: MemberPresence = serde_json::from_slice(&data)
                .map_err(|e| ProtocolError::Deserialization(format!("member presence: {e}")))?;
            Ok(Some(presence))
        }
        None => Ok(None),
    }
}

/// Write a member's presence to their SMPL subkey.
///
/// The caller must provide `SetDHTValueOptions` with the member's keypair
/// as the writer, since only the member can write to their own subkey.
pub async fn write_member_presence(
    dht: &DHTManager,
    key: &str,
    member_index: u32,
    presence: &MemberPresence,
    writer: veilid_core::KeyPair,
) -> Result<(), ProtocolError> {
    let subkey = member_subkey(member_index);
    let bytes = serde_json::to_vec(presence)
        .map_err(|e| ProtocolError::Serialization(format!("member presence: {e}")))?;
    dht.set_value_with_writer(key, subkey, bytes, writer).await
}

/// Watch all member presence subkeys for changes.
pub async fn watch_member_presence(
    dht: &DHTManager,
    key: &str,
    member_count: u32,
) -> Result<bool, ProtocolError> {
    let subkeys: Vec<u32> = (0..member_count)
        .map(member_subkey)
        .collect();
    if subkeys.is_empty() {
        return Ok(true);
    }
    dht.watch_record(key, &subkeys).await
}

// ── Pre-allocated SMPL slots (slot seed derivation) ──

/// Derive a deterministic Ed25519 keypair for a SMPL member slot.
///
/// Uses HKDF-SHA256(seed, "rekindle-slot-{index}") → 32 bytes → Ed25519 keypair.
/// Any admin with the slot seed can derive keypairs for all 256 slots.
pub fn derive_slot_keypair(seed: &[u8; 32], slot: u32) -> Result<ed25519_dalek::SigningKey, ProtocolError> {
    use hkdf::Hkdf;
    use sha2::Sha256;

    let hk = Hkdf::<Sha256>::new(None, seed);
    let info = format!("rekindle-slot-{slot}");
    let mut okm = [0u8; 32];
    hk.expand(info.as_bytes(), &mut okm)
        .map_err(|_| ProtocolError::CryptoError("HKDF expand failed".into()))?;
    Ok(ed25519_dalek::SigningKey::from_bytes(&okm))
}

/// Derive the Veilid `KeyPair` for a SMPL member slot.
///
/// Converts the Ed25519 keypair into a Veilid-compatible format for use
/// as a SMPL record writer.
pub fn derive_slot_veilid_keypair(
    seed: &[u8; 32],
    slot: u32,
) -> Result<veilid_core::KeyPair, ProtocolError> {
    let signing_key = derive_slot_keypair(seed, slot)?;
    let secret_bytes = signing_key.to_bytes();
    let public_bytes = signing_key.verifying_key().to_bytes();

    let bare_pub = veilid_core::BarePublicKey::new(&public_bytes);
    let bare_secret = veilid_core::BareSecretKey::new(&secret_bytes);
    let veilid_pubkey = veilid_core::PublicKey::new(veilid_core::CRYPTO_KIND_VLD0, bare_pub);

    Ok(veilid_core::KeyPair::new_from_parts(veilid_pubkey, bare_secret))
}

/// Create a pre-allocated registry segment with 256 SMPL slots.
///
/// Each slot's keypair is derived deterministically from the slot seed,
/// so any admin with the seed can process joins by assigning slots.
///
/// Returns `(record_key, owner_keypair)`.
pub async fn create_registry_segment(
    dht: &DHTManager,
    seed: &[u8; 32],
) -> Result<(String, Option<veilid_core::KeyPair>), ProtocolError> {
    let mut members = Vec::with_capacity(SLOTS_PER_SEGMENT as usize);
    for i in 0..SLOTS_PER_SEGMENT {
        let signing_key = derive_slot_keypair(seed, i)?;
        let public_bytes = signing_key.verifying_key().to_bytes();
        members.push(veilid_core::DHTSchemaSMPLMember {
            m_key: veilid_core::BareMemberId::new(&public_bytes),
            m_cnt: REGISTRY_MEMBER_SUBKEY_COUNT,
        });
    }

    let (key, owner_keypair) = dht
        .create_smpl_record(REGISTRY_OWNER_SUBKEY_COUNT, members)
        .await?;

    // Initialize empty member index and MEK vault
    let empty_index: Vec<MemberSummary> = Vec::new();
    write_member_index(dht, &key, &empty_index).await?;
    let empty_vault: Vec<MEKVaultEntry> = Vec::new();
    write_mek_vault(dht, &key, &empty_vault).await?;

    tracing::info!(key = %key, slots = SLOTS_PER_SEGMENT, "pre-allocated registry segment created");
    Ok((key, owner_keypair))
}

// ── Registry spine (manifest subkey 12) ──

/// Read the registry spine from the manifest record.
///
/// Returns `None` if the spine hasn't been written yet (single-segment community).
pub async fn read_registry_spine(
    dht: &DHTManager,
    manifest_key: &str,
) -> Result<Option<RegistrySpine>, ProtocolError> {
    match dht.get_value(manifest_key, MANIFEST_REGISTRY_SPINE).await? {
        Some(data) if !data.is_empty() => {
            let spine: RegistrySpine = serde_json::from_slice(&data)
                .map_err(|e| ProtocolError::Deserialization(format!("registry spine: {e}")))?;
            Ok(Some(spine))
        }
        _ => Ok(None),
    }
}

/// Write the registry spine to the manifest record (requires manifest_owner_keypair).
pub async fn write_registry_spine(
    dht: &DHTManager,
    manifest_key: &str,
    spine: &RegistrySpine,
) -> Result<(), ProtocolError> {
    let bytes = serde_json::to_vec(spine)
        .map_err(|e| ProtocolError::Serialization(format!("registry spine: {e}")))?;
    dht.set_value(manifest_key, MANIFEST_REGISTRY_SPINE, bytes).await
}

/// Build a `RegistrySpine` for a single-segment community.
pub fn single_segment_spine(record_key: &str, slot_seed_encrypted: Vec<u8>, member_count: u32) -> RegistrySpine {
    RegistrySpine {
        total_members: member_count,
        segments: vec![RegistrySegmentInfo {
            record_key: record_key.to_string(),
            slot_seed_encrypted,
            member_range: (0, member_count.saturating_sub(1)),
        }],
    }
}
