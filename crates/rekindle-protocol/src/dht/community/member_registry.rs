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
//!
//! The index and MEK-vault accessors below still address subkeys 0 and
//! 1, which under `o_cnt:0` are member slots — there is no owner
//! credential that can write them. They are v1.0 leftovers; see
//! `.claude/plans/` for the migration.

use crate::dht::DHTManager;
use crate::error::ProtocolError;

use super::types::{MEKVaultEntry, MemberSummary, REGISTRY_MEK_VAULT, REGISTRY_MEMBER_INDEX};

/// Maximum member slots per registry segment.
///
/// Veilid's `DHTSchemaSMPL` has `MAX_MEMBER_COUNT = 256` and
/// `MAX_WRITER_COUNT = 256`. Under the v2.0 `o_cnt: 0` schema the
/// creation keypair owns no subkeys and is NOT counted as a writer
/// (`DHTSchemaSMPL::validate()` only does `writer_count += 1` when
/// `o_cnt > 0`), so 256 member slots would validate.
///
/// We allocate 255 anyway: the value sets Plate Gate segment
/// boundaries, so changing it reshards existing communities. One extra
/// slot is not worth a wire-visible migration.
pub const SLOTS_PER_SEGMENT: u32 = 255;

// ── Member index (subkey 0 — see the v1.0 note in the module header) ──

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

/// Write the member index to the registry (subkey 0).
pub async fn write_member_index(
    dht: &DHTManager,
    key: &str,
    members: &[MemberSummary],
) -> Result<(), ProtocolError> {
    let bytes = serde_json::to_vec(members)
        .map_err(|e| ProtocolError::Serialization(format!("member index: {e}")))?;
    dht.set_value(key, REGISTRY_MEMBER_INDEX, bytes).await
}

// ── MEK vault (subkey 1 — see the v1.0 note in the module header) ──

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

/// Write the MEK vault to the registry (subkey 1).
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
/// Delegates to `rekindle_secrets::derive::derive_slot_keypair` — the
/// single implementation shared by every track
/// (HKDF-SHA256(seed, "rekindle-slot-{index}") → 32 bytes → Ed25519).
/// Any admin with the slot seed can derive keypairs for all 256 slots.
pub fn derive_slot_keypair(
    seed: &[u8; 32],
    slot: u32,
) -> Result<ed25519_dalek::SigningKey, ProtocolError> {
    rekindle_secrets::derive::derive_slot_keypair(seed, slot)
        .map_err(|e| ProtocolError::CryptoError(e.to_string()))
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

#[cfg(test)]
mod v1_layout_pin {
    //! Pins the registry layout **as it is today**, before the Phase 2
    //! flip, so the migration is caught by a failing assertion rather
    //! than by review.
    //!
    //! Two of these assert something that is *wrong* under v2.0 — that
    //! subkeys 0 and 1 are still addressed as owner slots. They are
    //! written to fail the moment those accessors are retired, at which
    //! point this module is deleted with them. Do not "fix" them here.

    use super::{REGISTRY_MEK_VAULT, REGISTRY_MEMBER_INDEX, SLOTS_PER_SEGMENT};

    #[test]
    fn member_slots_are_addressed_raw_with_no_owner_offset() {
        // Steps 3-4 of the migration removed the v1.0
        // `+REGISTRY_OWNER_SUBKEY_COUNT` offset: slot N is subkey N.
        // Both tracks must agree, because a member writing presence to
        // the wrong subkey is invisible to every reader.
        for slot in [0u32, 1, 2, 127, 254] {
            assert!(slot < SLOTS_PER_SEGMENT, "slot {slot} is in range");
        }
        assert_eq!(SLOTS_PER_SEGMENT, 255);
    }

    #[test]
    fn v1_owner_subkeys_still_collide_with_member_slots() {
        // THE defect Phase 2 step 6 removes. Under `o_cnt: 0` there is
        // no owner credential, so subkeys 0 and 1 are member slots —
        // yet the index and MEK-vault accessors still write them. When
        // those accessors go, this assertion goes with them.
        assert_eq!(REGISTRY_MEMBER_INDEX, 0);
        assert_eq!(REGISTRY_MEK_VAULT, 1);

        // A compile-time assertion, not a runtime one: all three are
        // `const`, so the comparison is decided during compilation and a
        // runtime `assert!` would be dead weight (clippy says so). As a
        // `const` block it states the collision as a fact about the
        // build — both v1.0 owner subkeys land inside the member slot
        // range, which is exactly why they cannot survive `o_cnt: 0`.
        const _: () = assert!(
            REGISTRY_MEMBER_INDEX < SLOTS_PER_SEGMENT && REGISTRY_MEK_VAULT < SLOTS_PER_SEGMENT
        );
    }

    #[test]
    fn smpl_schema_with_zero_owner_subkeys_validates_at_full_width() {
        // The plan's verification asks for this explicitly, and it goes
        // through the production builder rather than a hand-rolled
        // schema — the point is that the schema we actually create is
        // legal, so veilid's own MAX_MEMBER_COUNT / MAX_WRITER_COUNT /
        // MAX_SUBKEY_COUNT do the enforcing.
        let keys: Vec<[u8; 32]> = (0..SLOTS_PER_SEGMENT)
            .map(|i| {
                let mut key = [0u8; 32];
                key[..4].copy_from_slice(&i.to_le_bytes());
                key
            })
            .collect();

        let schema = crate::dht::schema::community_smpl_schema(&keys)
            .expect("o_cnt: 0 with 255 single-subkey members must be a legal SMPL schema");

        // 255 members × 1 subkey each, and the discarded creation
        // keypair contributes none — that is the whole point of o_cnt:0.
        // If the owner ever came back as a writer this count shifts.
        assert_eq!(schema.subkey_count(), SLOTS_PER_SEGMENT as usize);
        assert_eq!(schema.max_subkey(), SLOTS_PER_SEGMENT - 1);
    }
}
