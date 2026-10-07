//! Community-level keys: the per-member SMPL slot keypair, the registry
//! owner keypair and the slot seed. MEKs (community and channel scopes,
//! every generation) are in [`crate::typed::mek`].
//!
//! Persisting is best-effort: a failure is logged and the key lives in
//! memory until the next successful write.

use crate::{VaultKey, VaultStore};

/// Persist a community SMPL slot keypair to the keystore.
///
/// The slot keypair lets a member write their signed presence to their
/// assigned slot in the member registry SMPL record.
pub fn persist_slot_keypair(vault: &VaultStore, community_id: &str, keypair_str: &str) {
    if let Err(e) = vault.put(
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
pub fn load_slot_keypair(vault: &VaultStore, community_id: &str) -> Option<String> {
    match vault.get(&VaultKey::SlotKeypair {
        community: community_id.to_string(),
    }) {
        Ok(Some(bytes)) => String::from_utf8(bytes.to_vec()).ok(),
        Ok(None) => None,
        Err(e) => {
            tracing::trace!(error = %e, community = %community_id, "no slot keypair in keystore");
            None
        }
    }
}

/// Delete a community SMPL slot keypair from the keystore.
pub fn delete_slot_keypair(vault: &VaultStore, community_id: &str) {
    if let Err(e) = vault.delete(&VaultKey::SlotKeypair {
        community: community_id.to_string(),
    }) {
        tracing::warn!(error = %e, community = %community_id, "failed to delete slot keypair");
    }
}

/// Persist the registry owner keypair for a community to the open keystore.
pub fn persist_registry_keypair(vault: &VaultStore, community_id: &str, keypair_str: &str) {
    if let Err(e) = vault.put(
        &VaultKey::RegistryKeypair {
            community: community_id.to_string(),
        },
        keypair_str.as_bytes(),
    ) {
        tracing::warn!(error = %e, community = %community_id, "failed to persist registry keypair");
    }
}

/// Load the registry owner keypair for a community from the open keystore.
pub fn load_registry_keypair(vault: &VaultStore, community_id: &str) -> Option<String> {
    match vault.get(&VaultKey::RegistryKeypair {
        community: community_id.to_string(),
    }) {
        Ok(Some(bytes)) => String::from_utf8(bytes.to_vec()).ok(),
        Ok(None) => None,
        Err(e) => {
            tracing::trace!(error = %e, community = %community_id, "no registry keypair in keystore");
            None
        }
    }
}

/// Delete the registry owner keypair for a community from the open keystore.
pub fn delete_registry_keypair(vault: &VaultStore, community_id: &str) {
    if let Err(e) = vault.delete(&VaultKey::RegistryKeypair {
        community: community_id.to_string(),
    }) {
        tracing::warn!(error = %e, community = %community_id, "failed to delete registry keypair");
    }
}

/// Persist the slot seed (hex-encoded 32 bytes) for a community to the open keystore.
pub fn persist_slot_seed(vault: &VaultStore, community_id: &str, seed_hex: &str) {
    if let Err(e) = vault.put(
        &VaultKey::SlotSeed {
            community: community_id.to_string(),
        },
        seed_hex.as_bytes(),
    ) {
        tracing::warn!(error = %e, community = %community_id, "failed to persist slot seed");
    }
}

/// Load the slot seed for a community from the open keystore.
pub fn load_slot_seed(vault: &VaultStore, community_id: &str) -> Option<String> {
    match vault.get(&VaultKey::SlotSeed {
        community: community_id.to_string(),
    }) {
        Ok(Some(bytes)) => String::from_utf8(bytes.to_vec()).ok(),
        Ok(None) => None,
        Err(e) => {
            tracing::trace!(error = %e, community = %community_id, "no slot seed in keystore");
            None
        }
    }
}

/// Delete the slot seed for a community from the open keystore.
pub fn delete_slot_seed(vault: &VaultStore, community_id: &str) {
    if let Err(e) = vault.delete(&VaultKey::SlotSeed {
        community: community_id.to_string(),
    }) {
        tracing::warn!(error = %e, community = %community_id, "failed to delete slot seed");
    }
}
