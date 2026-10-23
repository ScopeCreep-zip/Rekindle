//! Community-level keys: the per-member SMPL slot keypair, the registry
//! owner keypair and the slot seed. MEKs (community and channel scopes,
//! every generation) are in [`crate::typed::mek`]. The member card's HPKE
//! secret (plan E3.2) is persisted strictly: the card publishes its public
//! half, so a secret not stored is a card no one can seal to.
//!
//! Persisting is best-effort: a failure is logged and the key lives in
//! memory until the next successful write.

use zeroize::Zeroizing;

use crate::{VaultError, VaultKey, VaultStore};

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

/// Store our member card's X25519 HPKE secret for `community_id`, before
/// the card that carries its public half is published.
///
/// # Errors
/// The vault write failed.
pub fn persist_member_hpke_secret(
    vault: &VaultStore,
    community_id: &str,
    secret: &[u8; 32],
) -> Result<(), VaultError> {
    vault.put(
        &VaultKey::MemberHpkeSecret {
            community: community_id.to_string(),
        },
        secret,
    )
}

/// Our member card's X25519 HPKE secret for `community_id`, if stored.
///
/// # Errors
/// The vault read failed, or the stored value is not 32 bytes.
pub fn load_member_hpke_secret(
    vault: &VaultStore,
    community_id: &str,
) -> Result<Option<Zeroizing<[u8; 32]>>, VaultError> {
    let Some(bytes) = vault.get(&VaultKey::MemberHpkeSecret {
        community: community_id.to_string(),
    })?
    else {
        return Ok(None);
    };
    let secret: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| VaultError::Schema(format!("member hpke secret is {} bytes", bytes.len())))?;
    Ok(Some(Zeroizing::new(secret)))
}

/// Forget our member card's HPKE secret for `community_id` — on leave.
pub fn delete_member_hpke_secret(vault: &VaultStore, community_id: &str) {
    if let Err(e) = vault.delete(&VaultKey::MemberHpkeSecret {
        community: community_id.to_string(),
    }) {
        tracing::warn!(error = %e, community = %community_id, "failed to delete member hpke secret");
    }
}
