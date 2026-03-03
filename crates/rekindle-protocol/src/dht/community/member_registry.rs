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
    MEKVaultEntry, MemberPresence, MemberSummary, REGISTRY_MEK_VAULT, REGISTRY_MEMBER_INDEX,
    REGISTRY_MEMBER_SUBKEY_COUNT, REGISTRY_OWNER_SUBKEY_COUNT,
};

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

/// Write the member index to the registry (coordinator only).
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

/// Write the MEK vault to the registry (coordinator only).
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
