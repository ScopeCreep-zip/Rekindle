//! Channel-level and per-generation MEK persistence.
//!
//! When MEK rotates (member departure), the old generation must remain in
//! the keystore so historical messages stay decryptable. Each generation gets
//! its own key: `mek_{community}_{channel}_{generation}`. A metadata key
//! `mek_generations_{community}_{channel}` tracks known generation numbers
//! (the vault has no prefix iteration).

use super::community_keys::{load_mek, persist_mek};
use super::StrongholdKeystore;
use rekindle_vault::VaultKey;

/// Persist a per-channel MEK to the open keystore.
///
/// Uses the key `mek_{community_id}_{channel_id}` under the communities namespace.
pub fn persist_channel_mek(
    keystore: &StrongholdKeystore,
    community_id: &str,
    channel_id: &str,
    mek: &rekindle_crypto::group::media_key::MediaEncryptionKey,
) {
    let payload = mek.to_wire_bytes();
    if let Err(e) = keystore.vault_put(
        &VaultKey::ChannelMek {
            community: community_id.to_string(),
            channel: channel_id.to_string(),
        },
        &payload,
    ) {
        tracing::warn!(
            error = %e, community = %community_id, channel = %channel_id,
            "failed to persist channel MEK to keystore"
        );
    } else {
        tracing::debug!(
            community = %community_id, channel = %channel_id,
            "channel MEK persisted to keystore"
        );
    }
}

/// Load a per-channel MEK from the open keystore.
///
/// Returns `Some(mek)` if successfully deserialized, `None` otherwise.
pub fn load_channel_mek(
    keystore: &StrongholdKeystore,
    community_id: &str,
    channel_id: &str,
) -> Option<rekindle_crypto::group::media_key::MediaEncryptionKey> {
    match keystore.vault_get(&VaultKey::ChannelMek {
        community: community_id.to_string(),
        channel: channel_id.to_string(),
    }) {
        Ok(Some(bytes)) => {
            rekindle_crypto::group::media_key::MediaEncryptionKey::from_wire_bytes(&bytes)
        }
        _ => None,
    }
}

/// Delete a per-channel MEK from the open keystore.
pub fn delete_channel_mek(keystore: &StrongholdKeystore, community_id: &str, channel_id: &str) {
    if let Err(e) = keystore.vault_delete(&VaultKey::ChannelMek {
        community: community_id.to_string(),
        channel: channel_id.to_string(),
    }) {
        tracing::warn!(
            error = %e, community = %community_id, channel = %channel_id,
            "failed to delete channel MEK from keystore"
        );
    } else {
        tracing::debug!(
            community = %community_id, channel = %channel_id,
            "channel MEK deleted from keystore"
        );
    }
}

/// Persist a channel MEK at a specific generation to the keystore.
///
/// Also updates the generations-list metadata key so `load_all_channel_mek_generations`
/// can enumerate all stored generations without prefix scanning.
pub fn persist_channel_mek_generation(
    keystore: &StrongholdKeystore,
    community_id: &str,
    channel_id: &str,
    mek: &rekindle_crypto::group::media_key::MediaEncryptionKey,
) {
    let generation = mek.generation();
    let payload = mek.to_wire_bytes();

    if let Err(e) = keystore.vault_put(
        &VaultKey::ChannelMekGeneration {
            community: community_id.to_string(),
            channel: channel_id.to_string(),
            generation,
        },
        &payload,
    ) {
        tracing::warn!(
            error = %e, community = %community_id, channel = %channel_id, generation,
            "failed to persist channel MEK generation to keystore"
        );
        return;
    }

    // Update the generations index
    update_generations_index(keystore, community_id, channel_id, generation);

    // Also update the "latest" key for backward compatibility with existing code
    let _ = keystore.vault_put(
        &VaultKey::ChannelMek {
            community: community_id.to_string(),
            channel: channel_id.to_string(),
        },
        &payload,
    );

    tracing::debug!(
        community = %community_id, channel = %channel_id, generation,
        "channel MEK generation persisted to keystore"
    );
}

/// Persist either a community-level or channel-level MEK.
pub fn store_mek(
    keystore: &StrongholdKeystore,
    community_id: &str,
    channel_id: Option<&str>,
    mek: &rekindle_crypto::group::media_key::MediaEncryptionKey,
) {
    if let Some(channel_id) = channel_id {
        persist_channel_mek_generation(keystore, community_id, channel_id, mek);
    } else {
        persist_mek(keystore, community_id, mek);
    }
}

/// Load a specific MEK generation for a channel from the keystore.
pub fn load_channel_mek_generation(
    keystore: &StrongholdKeystore,
    community_id: &str,
    channel_id: &str,
    generation: u64,
) -> Option<rekindle_crypto::group::media_key::MediaEncryptionKey> {
    match keystore.vault_get(&VaultKey::ChannelMekGeneration {
        community: community_id.to_string(),
        channel: channel_id.to_string(),
        generation,
    }) {
        Ok(Some(bytes)) => {
            rekindle_crypto::group::media_key::MediaEncryptionKey::from_wire_bytes(&bytes)
        }
        _ => None,
    }
}

/// Load all persisted MEK generations for a channel from the keystore.
///
/// Returns MEKs sorted by generation (ascending). Used on startup to
/// populate `channel_mek_cache` so historical messages remain decryptable.
pub fn load_all_channel_mek_generations(
    keystore: &StrongholdKeystore,
    community_id: &str,
    channel_id: &str,
) -> Vec<rekindle_crypto::group::media_key::MediaEncryptionKey> {
    let generations = load_generations_index(keystore, community_id, channel_id);
    let mut meks = Vec::new();
    for gen in generations {
        if let Some(mek) = load_channel_mek_generation(keystore, community_id, channel_id, gen) {
            meks.push(mek);
        }
    }
    meks.sort_by_key(rekindle_crypto::group::media_key::MediaEncryptionKey::generation);
    meks
}

/// Load all known MEKs for a community or channel.
pub fn load_all_meks(
    keystore: &StrongholdKeystore,
    community_id: &str,
    channel_id: Option<&str>,
) -> Vec<rekindle_crypto::group::media_key::MediaEncryptionKey> {
    if let Some(channel_id) = channel_id {
        return load_all_channel_mek_generations(keystore, community_id, channel_id);
    }
    load_mek(keystore, community_id).into_iter().collect()
}

/// Update the generations-index metadata key with a new generation number.
fn update_generations_index(
    keystore: &StrongholdKeystore,
    community_id: &str,
    channel_id: &str,
    generation: u64,
) {
    let mut generations = load_generations_index(keystore, community_id, channel_id);

    if !generations.contains(&generation) {
        generations.push(generation);
        generations.sort_unstable();
    }

    // Serialize as JSON array of u64
    let payload = serde_json::to_vec(&generations).unwrap_or_default();
    if let Err(e) = keystore.vault_put(
        &VaultKey::ChannelMekGenerationsIndex {
            community: community_id.to_string(),
            channel: channel_id.to_string(),
        },
        &payload,
    ) {
        tracing::warn!(
            error = %e, community = %community_id, channel = %channel_id,
            "failed to update MEK generations index"
        );
    }
}

/// Load the generations-index for a channel from the keystore.
fn load_generations_index(
    keystore: &StrongholdKeystore,
    community_id: &str,
    channel_id: &str,
) -> Vec<u64> {
    match keystore.vault_get(&VaultKey::ChannelMekGenerationsIndex {
        community: community_id.to_string(),
        channel: channel_id.to_string(),
    }) {
        Ok(Some(bytes)) => serde_json::from_slice(&bytes).unwrap_or_default(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use crate::keystore::*;
    use rekindle_crypto::group::media_key::MediaEncryptionKey;
    use tempfile::TempDir;

    #[test]
    fn per_generation_mek_roundtrip() {
        let dir = TempDir::new().unwrap();
        let ks = StrongholdKeystore::initialize(dir.path(), "testpass").unwrap();

        let mek_gen1 = MediaEncryptionKey::generate(1);
        let mek_gen2 = MediaEncryptionKey::generate(2);

        // Persist two generations
        persist_channel_mek_generation(&ks, "comm_a", "ch_01", &mek_gen1);
        persist_channel_mek_generation(&ks, "comm_a", "ch_01", &mek_gen2);

        // Load specific generation
        let loaded = load_channel_mek_generation(&ks, "comm_a", "ch_01", 1).unwrap();
        assert_eq!(loaded.generation(), 1);
        assert_eq!(loaded.as_bytes(), mek_gen1.as_bytes());

        let loaded = load_channel_mek_generation(&ks, "comm_a", "ch_01", 2).unwrap();
        assert_eq!(loaded.generation(), 2);
        assert_eq!(loaded.as_bytes(), mek_gen2.as_bytes());

        // Nonexistent generation returns None
        assert!(load_channel_mek_generation(&ks, "comm_a", "ch_01", 99).is_none());
    }

    #[test]
    fn load_all_generations_returns_sorted() {
        let dir = TempDir::new().unwrap();
        let ks = StrongholdKeystore::initialize(dir.path(), "testpass").unwrap();

        // Persist out of order
        let mek3 = MediaEncryptionKey::generate(3);
        let mek1 = MediaEncryptionKey::generate(1);
        let mek2 = MediaEncryptionKey::generate(2);

        persist_channel_mek_generation(&ks, "comm_b", "ch_02", &mek3);
        persist_channel_mek_generation(&ks, "comm_b", "ch_02", &mek1);
        persist_channel_mek_generation(&ks, "comm_b", "ch_02", &mek2);

        let all = load_all_channel_mek_generations(&ks, "comm_b", "ch_02");
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].generation(), 1);
        assert_eq!(all[1].generation(), 2);
        assert_eq!(all[2].generation(), 3);
    }

    #[test]
    fn per_generation_survives_reopen() {
        let dir = TempDir::new().unwrap();
        let mek = MediaEncryptionKey::generate(42);
        let key_bytes = mek.as_bytes().to_vec();

        // Persist and close
        {
            let ks = StrongholdKeystore::initialize(dir.path(), "pass").unwrap();
            persist_channel_mek_generation(&ks, "comm_c", "ch_03", &mek);
        }

        // Reopen and verify
        {
            let ks = StrongholdKeystore::initialize(dir.path(), "pass").unwrap();
            let loaded = load_channel_mek_generation(&ks, "comm_c", "ch_03", 42).unwrap();
            assert_eq!(loaded.generation(), 42);
            assert_eq!(loaded.as_bytes(), key_bytes.as_slice());
        }
    }

    #[test]
    fn different_channels_isolated() {
        let dir = TempDir::new().unwrap();
        let ks = StrongholdKeystore::initialize(dir.path(), "testpass").unwrap();

        let mek_a = MediaEncryptionKey::generate(1);
        let mek_b = MediaEncryptionKey::generate(1);

        persist_channel_mek_generation(&ks, "comm_d", "ch_alpha", &mek_a);
        persist_channel_mek_generation(&ks, "comm_d", "ch_beta", &mek_b);

        // Each channel has its own generation namespace
        let all_alpha = load_all_channel_mek_generations(&ks, "comm_d", "ch_alpha");
        let all_beta = load_all_channel_mek_generations(&ks, "comm_d", "ch_beta");

        assert_eq!(all_alpha.len(), 1);
        assert_eq!(all_beta.len(), 1);
        assert_eq!(all_alpha[0].as_bytes(), mek_a.as_bytes());
        assert_eq!(all_beta[0].as_bytes(), mek_b.as_bytes());
    }

    #[test]
    fn load_all_meks_wraps_community_and_channel_variants() {
        let dir = TempDir::new().unwrap();
        let ks = StrongholdKeystore::initialize(dir.path(), "testpass").unwrap();

        let community_mek = MediaEncryptionKey::generate(3);
        let channel_mek_a = MediaEncryptionKey::generate(1);
        let channel_mek_b = MediaEncryptionKey::generate(2);

        persist_mek(&ks, "comm_e", &community_mek);
        persist_channel_mek_generation(&ks, "comm_e", "ch_01", &channel_mek_b);
        persist_channel_mek_generation(&ks, "comm_e", "ch_01", &channel_mek_a);

        let community_all = load_all_meks(&ks, "comm_e", None);
        assert_eq!(community_all.len(), 1);
        assert_eq!(community_all[0].generation(), 3);

        let channel_all = load_all_meks(&ks, "comm_e", Some("ch_01"));
        assert_eq!(channel_all.len(), 2);
        assert_eq!(channel_all[0].generation(), 1);
        assert_eq!(channel_all[1].generation(), 2);
    }
}
