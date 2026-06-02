//! Community-level key persistence: the community MEK plus the per-member
//! SMPL slot keypair, registry owner keypair, and slot seed. Channel-level
//! and per-generation MEK persistence lives in [`super::channel_mek`].

use super::StrongholdKeystore;
use rekindle_vault::VaultKey;

/// Persist a community's MEK to the open keystore.
///
/// Serializes as `generation (8 bytes LE) + key (32 bytes)`, stores under
/// `communities / mek_<community_id>`.
pub fn persist_mek(
    keystore: &StrongholdKeystore,
    community_id: &str,
    mek: &rekindle_crypto::group::media_key::MediaEncryptionKey,
) {
    let payload = mek.to_wire_bytes();
    if let Err(e) = keystore.vault_put(
        &VaultKey::CommunityMek {
            community: community_id.to_string(),
        },
        &payload,
    ) {
        tracing::warn!(error = %e, community = %community_id, "failed to persist MEK to keystore");
    } else {
        tracing::debug!(community = %community_id, "MEK persisted to keystore");
    }
}

/// Strict variant of `persist_mek` for command handlers that must surface
/// failure to the frontend instead of silently logging.
///
/// Architecture §17 (community/MEK durability across restarts): the cached
/// MEK lives in `state.mek_cache` until persisted. If the keystore write
/// fails, the MEK survives only in memory — on the next app restart the user
/// can't decrypt channel messages from this generation and has to wait for an
/// MEK request/cascade. Vulnerable users need to know this happened, not see
/// a silent "Joined!" toast. Caller
/// (`commands::community::crud::join_community`) propagates the `Err` through
/// the Tauri IPC boundary; the frontend's toast (A6 fix at
/// handlers/community.handlers.ts:120-126) renders the message.
pub fn persist_mek_strict(
    keystore: &StrongholdKeystore,
    community_id: &str,
    mek: &rekindle_crypto::group::media_key::MediaEncryptionKey,
) -> Result<(), String> {
    let payload = mek.to_wire_bytes();
    keystore
        .vault_put(
            &VaultKey::CommunityMek {
                community: community_id.to_string(),
            },
            &payload,
        )
        .map_err(|e| {
            format!("Keystore locked or busy — MEK not persisted (will be lost on restart): {e}")
        })?;
    tracing::debug!(community = %community_id, "MEK persisted to keystore (strict)");
    Ok(())
}

/// Load a community's MEK from the open keystore.
///
/// Returns `Some(mek)` if successfully deserialized, `None` otherwise.
pub fn load_mek(
    keystore: &StrongholdKeystore,
    community_id: &str,
) -> Option<rekindle_crypto::group::media_key::MediaEncryptionKey> {
    match keystore.vault_get(&VaultKey::CommunityMek {
        community: community_id.to_string(),
    }) {
        Ok(Some(bytes)) => {
            rekindle_crypto::group::media_key::MediaEncryptionKey::from_wire_bytes(&bytes)
        }
        _ => None,
    }
}

/// Delete a community's MEK from the open keystore.
pub fn delete_mek(keystore: &StrongholdKeystore, community_id: &str) {
    if let Err(e) = keystore.vault_delete(&VaultKey::CommunityMek {
        community: community_id.to_string(),
    }) {
        tracing::warn!(error = %e, community = %community_id, "failed to delete MEK from keystore");
    } else {
        tracing::debug!(community = %community_id, "MEK deleted from keystore");
    }
}

/// Persist a community SMPL slot keypair to the keystore.
///
/// The slot keypair lets a member write their signed presence to their
/// assigned slot in the member registry SMPL record.
pub fn persist_slot_keypair(keystore: &StrongholdKeystore, community_id: &str, keypair_str: &str) {
    if let Err(e) = keystore.vault_put(
        &VaultKey::SlotKeypair {
            community: community_id.to_string(),
        },
        keypair_str.as_bytes(),
    ) {
        tracing::warn!(error = %e, community = %community_id, "failed to persist slot keypair");
    } else {
        tracing::debug!(community = %community_id, "slot keypair persisted to keystore");
    }
}

/// Load a community SMPL slot keypair from the keystore.
pub fn load_slot_keypair(keystore: &StrongholdKeystore, community_id: &str) -> Option<String> {
    match keystore.vault_get(&VaultKey::SlotKeypair {
        community: community_id.to_string(),
    }) {
        Ok(Some(bytes)) => String::from_utf8(bytes).ok(),
        Ok(None) => None,
        Err(e) => {
            tracing::trace!(error = %e, community = %community_id, "no slot keypair in keystore");
            None
        }
    }
}

/// Delete a community SMPL slot keypair from the keystore.
pub fn delete_slot_keypair(keystore: &StrongholdKeystore, community_id: &str) {
    if let Err(e) = keystore.vault_delete(&VaultKey::SlotKeypair {
        community: community_id.to_string(),
    }) {
        tracing::warn!(error = %e, community = %community_id, "failed to delete slot keypair");
    }
}

/// Persist the registry owner keypair for a community to the open keystore.
pub fn persist_registry_keypair(
    keystore: &StrongholdKeystore,
    community_id: &str,
    keypair_str: &str,
) {
    if let Err(e) = keystore.vault_put(
        &VaultKey::RegistryKeypair {
            community: community_id.to_string(),
        },
        keypair_str.as_bytes(),
    ) {
        tracing::warn!(error = %e, community = %community_id, "failed to persist registry keypair");
    }
}

/// Load the registry owner keypair for a community from the open keystore.
pub fn load_registry_keypair(keystore: &StrongholdKeystore, community_id: &str) -> Option<String> {
    match keystore.vault_get(&VaultKey::RegistryKeypair {
        community: community_id.to_string(),
    }) {
        Ok(Some(bytes)) => String::from_utf8(bytes).ok(),
        Ok(None) => None,
        Err(e) => {
            tracing::trace!(error = %e, community = %community_id, "no registry keypair in keystore");
            None
        }
    }
}

/// Delete the registry owner keypair for a community from the open keystore.
pub fn delete_registry_keypair(keystore: &StrongholdKeystore, community_id: &str) {
    if let Err(e) = keystore.vault_delete(&VaultKey::RegistryKeypair {
        community: community_id.to_string(),
    }) {
        tracing::warn!(error = %e, community = %community_id, "failed to delete registry keypair");
    }
}

/// Persist the slot seed (hex-encoded 32 bytes) for a community to the open keystore.
pub fn persist_slot_seed(keystore: &StrongholdKeystore, community_id: &str, seed_hex: &str) {
    if let Err(e) = keystore.vault_put(
        &VaultKey::SlotSeed {
            community: community_id.to_string(),
        },
        seed_hex.as_bytes(),
    ) {
        tracing::warn!(error = %e, community = %community_id, "failed to persist slot seed");
    }
}

/// Load the slot seed for a community from the open keystore.
pub fn load_slot_seed(keystore: &StrongholdKeystore, community_id: &str) -> Option<String> {
    match keystore.vault_get(&VaultKey::SlotSeed {
        community: community_id.to_string(),
    }) {
        Ok(Some(bytes)) => String::from_utf8(bytes).ok(),
        Ok(None) => None,
        Err(e) => {
            tracing::trace!(error = %e, community = %community_id, "no slot seed in keystore");
            None
        }
    }
}

/// Delete the slot seed for a community from the open keystore.
pub fn delete_slot_seed(keystore: &StrongholdKeystore, community_id: &str) {
    if let Err(e) = keystore.vault_delete(&VaultKey::SlotSeed {
        community: community_id.to_string(),
    }) {
        tracing::warn!(error = %e, community = %community_id, "failed to delete slot seed");
    }
}

// persist_channel_log_keypair, load_channel_log_keypair, delete_channel_log_keypair
// removed — SMPL channel records use the shared slot seed. Members derive their
// writer keypair via derive_slot_veilid_keypair(seed, slot_index).
