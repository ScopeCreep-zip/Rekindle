//! Member registry SMPL record operations.
//!
//! The member registry is a SMPL schema record where:
//! - Owner controls subkeys 0-1 (member index + MEK vault)
//! - Each member gets 1 subkey for their presence data
//! - Member subkeys start at offset 2 (REGISTRY_OWNER_SUBKEY_COUNT)

use veilid_core::{DHTSchemaSMPLMember, KeyPair, RoutingContext};

use super::record;
use crate::error::{TransportError, Result};
use crate::payload::dht_types::{
    MekVaultEntry, MemberPresence, MemberSummary, REGISTRY_MEK_VAULT,
    REGISTRY_MEMBER_INDEX, REGISTRY_MEMBER_SUBKEY_COUNT, REGISTRY_OWNER_SUBKEY_COUNT,
    SLOTS_PER_SEGMENT,
};

/// Calculate the DHT subkey index for a member given their slot index.
pub fn member_subkey(slot_index: u32) -> u32 {
    u32::from(REGISTRY_OWNER_SUBKEY_COUNT) + slot_index
}

/// Operations on a community member registry.
pub struct RegistryOps<'a> {
    rc: &'a RoutingContext,
}

impl<'a> RegistryOps<'a> {
    pub fn new(rc: &'a RoutingContext) -> Self {
        Self { rc }
    }

    /// Create a new empty registry (DFLT with owner subkeys only).
    pub async fn create(&self) -> Result<(String, Option<KeyPair>)> {
        let (key, keypair) = record::create_dflt(
            self.rc,
            REGISTRY_OWNER_SUBKEY_COUNT,
            None,
        ).await?;

        self.write_member_index(&key, &[]).await?;
        self.write_mek_vault(&key, &[]).await?;

        tracing::info!(key = %key, "member registry created");
        Ok((key, keypair))
    }

    /// Create a SMPL registry with pre-allocated member slots.
    pub async fn create_with_members(
        &self,
        members: Vec<DHTSchemaSMPLMember>,
        initial_index: &[MemberSummary],
    ) -> Result<(String, Option<KeyPair>)> {
        let (key, keypair) = record::create_smpl(
            self.rc,
            REGISTRY_OWNER_SUBKEY_COUNT,
            members,
        ).await?;

        self.write_member_index(&key, initial_index).await?;
        self.write_mek_vault(&key, &[]).await?;

        tracing::info!(key = %key, members = initial_index.len(), "SMPL registry created");
        Ok((key, keypair))
    }

    /// Create a pre-allocated registry segment with 255 derived SMPL slots.
    pub async fn create_segment(&self, seed: &[u8; 32]) -> Result<(String, Option<KeyPair>)> {
        let mut members = Vec::with_capacity(SLOTS_PER_SEGMENT as usize);
        for i in 0..SLOTS_PER_SEGMENT {
            let signing_key = derive_slot_keypair(seed, i)?;
            let pub_bytes = signing_key.verifying_key().to_bytes();
            members.push(DHTSchemaSMPLMember {
                m_key: veilid_core::BareMemberId::new(&pub_bytes),
                m_cnt: REGISTRY_MEMBER_SUBKEY_COUNT,
            });
        }

        let (key, keypair) = record::create_smpl(
            self.rc,
            REGISTRY_OWNER_SUBKEY_COUNT,
            members,
        ).await?;

        self.write_member_index(&key, &[]).await?;
        self.write_mek_vault(&key, &[]).await?;

        tracing::info!(key = %key, slots = SLOTS_PER_SEGMENT, "registry segment created");
        Ok((key, keypair))
    }

    // ── Member index (owner subkey 0) ────────────────────────────

    pub async fn read_member_index(&self, key: &str) -> Result<Vec<MemberSummary>> {
        match record::get(self.rc, key, REGISTRY_MEMBER_INDEX, false).await? {
            Some(data) => serde_json::from_slice(&data).map_err(|e| {
                TransportError::DeserializationFailed { type_id: 0, reason: format!("index: {e}") }
            }),
            None => Ok(Vec::new()),
        }
    }

    pub async fn write_member_index(&self, key: &str, members: &[MemberSummary]) -> Result<()> {
        let bytes = serde_json::to_vec(members)
            .map_err(|e| TransportError::SerializationFailed { reason: format!("index: {e}") })?;
        record::set(self.rc, key, REGISTRY_MEMBER_INDEX, bytes, None).await
    }

    // ── MEK vault (owner subkey 1) ───────────────────────────────

    pub async fn read_mek_vault(&self, key: &str) -> Result<Vec<MekVaultEntry>> {
        match record::get(self.rc, key, REGISTRY_MEK_VAULT, false).await? {
            Some(data) => serde_json::from_slice(&data).map_err(|e| {
                TransportError::DeserializationFailed { type_id: 0, reason: format!("vault: {e}") }
            }),
            None => Ok(Vec::new()),
        }
    }

    pub async fn write_mek_vault(&self, key: &str, vault: &[MekVaultEntry]) -> Result<()> {
        let bytes = serde_json::to_vec(vault)
            .map_err(|e| TransportError::SerializationFailed { reason: format!("vault: {e}") })?;
        record::set(self.rc, key, REGISTRY_MEK_VAULT, bytes, None).await
    }

    // ── Member presence (member subkeys) ─────────────────────────

    pub async fn read_presence(
        &self,
        key: &str,
        slot_index: u32,
        force_refresh: bool,
    ) -> Result<Option<MemberPresence>> {
        let subkey = member_subkey(slot_index);
        match record::get(self.rc, key, subkey, force_refresh).await? {
            Some(data) => {
                let presence: MemberPresence = serde_json::from_slice(&data).map_err(|e| {
                    TransportError::DeserializationFailed { type_id: 0, reason: format!("presence: {e}") }
                })?;
                Ok(Some(presence))
            }
            None => Ok(None),
        }
    }

    pub async fn write_presence(
        &self,
        key: &str,
        slot_index: u32,
        presence: &MemberPresence,
        writer: KeyPair,
    ) -> Result<()> {
        let subkey = member_subkey(slot_index);
        let bytes = serde_json::to_vec(presence)
            .map_err(|e| TransportError::SerializationFailed { reason: format!("presence: {e}") })?;
        record::set(self.rc, key, subkey, bytes, Some(writer)).await
    }

    // ── Watch ────────────────────────────────────────────────────

    pub async fn watch_presence(&self, key: &str, member_count: u32) -> Result<bool> {
        let subkeys: Vec<u32> = (0..member_count).map(member_subkey).collect();
        if subkeys.is_empty() { return Ok(true); }
        record::watch(self.rc, key, &subkeys).await
    }

    // ── Open / Close ─────────────────────────────────────────────

    pub async fn open_writable(&self, key: &str, writer: KeyPair) -> Result<()> {
        record::open_writable(self.rc, key, writer).await
    }

    pub async fn open_readonly(&self, key: &str) -> Result<()> {
        record::open_readonly(self.rc, key).await
    }

    pub async fn close(&self, key: &str) -> Result<()> {
        record::close(self.rc, key).await
    }
}

// ── Slot keypair derivation ──────────────────────────────────────────

/// Derive a deterministic Ed25519 keypair for a SMPL member slot.
///
/// Uses HKDF-SHA256(seed, "rekindle-slot-{index}") -> 32 bytes -> Ed25519.
pub fn derive_slot_keypair(
    seed: &[u8; 32],
    slot: u32,
) -> Result<ed25519_dalek::SigningKey> {
    use hkdf::Hkdf;
    use sha2::Sha256;

    let hk = Hkdf::<Sha256>::new(None, seed);
    let info = format!("rekindle-slot-{slot}");
    let mut okm = [0u8; 32];
    hk.expand(info.as_bytes(), &mut okm)
        .map_err(|_| TransportError::Internal("HKDF expand failed".into()))?;
    Ok(ed25519_dalek::SigningKey::from_bytes(&okm))
}

/// Derive the Veilid KeyPair for a SMPL member slot.
pub fn derive_slot_veilid_keypair(seed: &[u8; 32], slot: u32) -> Result<KeyPair> {
    let signing_key = derive_slot_keypair(seed, slot)?;
    let pub_bytes = signing_key.verifying_key().to_bytes();
    let secret_bytes = signing_key.to_bytes();

    let bare_pub = veilid_core::BarePublicKey::new(&pub_bytes);
    let bare_secret = veilid_core::BareSecretKey::new(&secret_bytes);
    let veilid_pubkey = veilid_core::PublicKey::new(veilid_core::CRYPTO_KIND_VLD0, bare_pub);

    Ok(KeyPair::new_from_parts(veilid_pubkey, bare_secret))
}
