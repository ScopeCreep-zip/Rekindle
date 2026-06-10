//! Member registry operations for SMPL multi-writer DHT records.
//!
//! Communities v2.0 registry layout (`DHTSchema::smpl(0, members)` —
//! architecture "no node above another"): the creation keypair is
//! discarded after genesis and there are NO owner subkeys. Every
//! subkey 0..254 is a member slot; members claim a slot
//! self-sovereignly (join flow) and write their own `MemberPresence`
//! to it with the slot keypair derived from the shared slot seed.
//!
//! Presence reads/writes address slots RAW — the v1.0 coordinator-era
//! `+REGISTRY_OWNER_SUBKEY_COUNT` offset is gone. Record creation
//! lives in the governance runtime (`create_smpl_record` with
//! `community_smpl_schema`); this module keeps the slot-keypair
//! derivation and the index/MEK-vault accessors still consumed by the
//! governance + MEK rotation adapters.

use crate::dht::DHTManager;
use crate::error::ProtocolError;

use super::types::{MEKVaultEntry, MemberSummary, REGISTRY_MEK_VAULT, REGISTRY_MEMBER_INDEX};

/// Maximum member slots per registry segment.
///
/// Veilid's `DHTSchemaSMPL` has `MAX_MEMBER_COUNT = 256` and `MAX_WRITER_COUNT = 256`.
/// Since the owner counts as 1 writer, we can have at most 255 member writers
/// (1 owner + 255 members = 256 total writers).
pub const SLOTS_PER_SEGMENT: u32 = 255;

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

// ── Pre-allocated SMPL slots (slot seed derivation) ──

/// Derive a deterministic Ed25519 keypair for a SMPL member slot.
///
/// Uses HKDF-SHA256(seed, "rekindle-slot-{index}") → 32 bytes → Ed25519 keypair.
/// Any admin with the slot seed can derive keypairs for all 256 slots.
pub fn derive_slot_keypair(
    seed: &[u8; 32],
    slot: u32,
) -> Result<ed25519_dalek::SigningKey, ProtocolError> {
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

    Ok(veilid_core::KeyPair::new_from_parts(
        veilid_pubkey,
        bare_secret,
    ))
}
