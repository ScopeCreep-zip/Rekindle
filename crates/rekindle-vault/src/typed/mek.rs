//! MEK persistence, per scope and generation.
//!
//! Every key a member installs is stored under its exact
//! `(community, scope, generation)` so history written under a replaced key
//! stays readable (the Megolm rule: receivers keep superseded session
//! keys). Each scope also keeps its latest key, read at login, and an index
//! of stored generations — the vault has no prefix iteration, and leaving a
//! community must erase every generation it holds.

use rekindle_secrets::keys::MediaEncryptionKey;
use rekindle_types::channel_keys::KeyScope;

use crate::{VaultKey, VaultStore};

fn latest_key(community_id: &str, scope: KeyScope) -> VaultKey {
    match scope {
        KeyScope::Community => VaultKey::CommunityMek {
            community: community_id.to_string(),
        },
        KeyScope::Channel(channel) => VaultKey::ChannelMek {
            community: community_id.to_string(),
            channel: channel.to_hex(),
        },
    }
}

fn generation_key(community_id: &str, scope: KeyScope, generation: u64) -> VaultKey {
    match scope {
        KeyScope::Community => VaultKey::CommunityMekGeneration {
            community: community_id.to_string(),
            generation,
        },
        KeyScope::Channel(channel) => VaultKey::ChannelMekGeneration {
            community: community_id.to_string(),
            channel: channel.to_hex(),
            generation,
        },
    }
}

fn index_key(community_id: &str, scope: KeyScope) -> VaultKey {
    match scope {
        KeyScope::Community => VaultKey::CommunityMekGenerationsIndex {
            community: community_id.to_string(),
        },
        KeyScope::Channel(channel) => VaultKey::ChannelMekGenerationsIndex {
            community: community_id.to_string(),
            channel: channel.to_hex(),
        },
    }
}

fn load(vault: &VaultStore, key: &VaultKey) -> Option<MediaEncryptionKey> {
    match vault.get(key) {
        Ok(Some(bytes)) => MediaEncryptionKey::from_wire_bytes(&bytes),
        _ => None,
    }
}

/// The scope's stored generations. A read or decode failure is an error,
/// not an empty list: rewriting the index from nothing would lose track of
/// generations that leaving the community must erase.
fn load_index(vault: &VaultStore, community_id: &str, scope: KeyScope) -> Result<Vec<u64>, String> {
    match vault
        .get(&index_key(community_id, scope))
        .map_err(|e| format!("{scope} MEK generations index: {e}"))?
    {
        Some(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| format!("{scope} MEK generations index is corrupt: {e}")),
        None => Ok(Vec::new()),
    }
}

/// Store `mek` under its generation, record the generation in the scope's
/// index, and make it the scope's latest key unless a newer one is stored.
///
/// # Errors
/// The vault write that failed. The key then lives only in memory and is
/// lost at restart, so callers that answer a user surface it.
pub fn persist_mek(
    vault: &VaultStore,
    community_id: &str,
    scope: KeyScope,
    mek: &MediaEncryptionKey,
) -> Result<(), String> {
    let generation = mek.generation();
    let payload = mek.to_wire_bytes();
    let failed = |e: crate::VaultError| {
        format!("Keystore locked or busy — {scope} MEK generation {generation} not persisted (will be lost on restart): {e}")
    };

    vault
        .put(&generation_key(community_id, scope, generation), &payload)
        .map_err(failed)?;

    let mut generations = load_index(vault, community_id, scope)?;
    if !generations.contains(&generation) {
        generations.push(generation);
        generations.sort_unstable();
        let index =
            serde_json::to_vec(&generations).map_err(|e| format!("MEK generations index: {e}"))?;
        vault
            .put(&index_key(community_id, scope), &index)
            .map_err(failed)?;
    }

    let latest = latest_key(community_id, scope);
    let newer_stored = load(vault, &latest).is_some_and(|m| m.generation() > generation);
    if !newer_stored {
        vault.put(&latest, &payload).map_err(failed)?;
    }
    tracing::debug!(community = %community_id, %scope, generation, "MEK persisted to keystore");
    Ok(())
}

/// The scope's latest stored key (login restore).
pub fn load_latest_mek(
    vault: &VaultStore,
    community_id: &str,
    scope: KeyScope,
) -> Option<MediaEncryptionKey> {
    load(vault, &latest_key(community_id, scope))
}

/// The scope's stored key at exactly `generation`.
pub fn load_mek_generation(
    vault: &VaultStore,
    community_id: &str,
    scope: KeyScope,
    generation: u64,
) -> Option<MediaEncryptionKey> {
    load(vault, &generation_key(community_id, scope, generation))
        .filter(|mek| mek.generation() == generation)
}

/// Erase every stored generation of the scope, its index and its latest
/// key (leaving a community).
pub fn delete_scope_meks(vault: &VaultStore, community_id: &str, scope: KeyScope) {
    let generations = load_index(vault, community_id, scope).unwrap_or_else(|e| {
        tracing::error!(community = %community_id, %scope, error = %e, "superseded MEK generations cannot be listed and stay in the vault");
        Vec::new()
    });
    let mut keys: Vec<VaultKey> = generations
        .into_iter()
        .map(|generation| generation_key(community_id, scope, generation))
        .collect();
    keys.push(index_key(community_id, scope));
    keys.push(latest_key(community_id, scope));
    for key in &keys {
        if let Err(e) = vault.delete(key) {
            tracing::warn!(error = %e, community = %community_id, %scope, "failed to delete MEK from keystore");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rekindle_types::id::ChannelId;
    use tempfile::TempDir;

    const CH: KeyScope = KeyScope::Channel(ChannelId([1; 16]));
    const OTHER: KeyScope = KeyScope::Channel(ChannelId([2; 16]));

    fn keystore(dir: &TempDir) -> VaultStore {
        VaultStore::open(&dir.path().join("rekindle.vault"), "testpass").unwrap()
    }

    #[test]
    fn every_generation_of_every_scope_is_kept() {
        let dir = TempDir::new().unwrap();
        let ks = keystore(&dir);
        let (g1, g2) = (
            MediaEncryptionKey::generate(1),
            MediaEncryptionKey::generate(2),
        );
        for scope in [KeyScope::Community, CH] {
            persist_mek(&ks, "c", scope, &g1).unwrap();
            persist_mek(&ks, "c", scope, &g2).unwrap();
            assert_eq!(
                load_mek_generation(&ks, "c", scope, 1).unwrap().as_bytes(),
                g1.as_bytes()
            );
            assert_eq!(
                load_mek_generation(&ks, "c", scope, 2).unwrap().as_bytes(),
                g2.as_bytes()
            );
            assert_eq!(load_latest_mek(&ks, "c", scope).unwrap().generation(), 2);
            assert!(load_mek_generation(&ks, "c", scope, 3).is_none());
        }
        assert!(load_latest_mek(&ks, "c", OTHER).is_none());
    }

    /// Storing an older generation late keeps it readable but does not
    /// roll the scope's latest key back.
    #[test]
    fn late_older_generation_does_not_replace_latest() {
        let dir = TempDir::new().unwrap();
        let ks = keystore(&dir);
        persist_mek(
            &ks,
            "c",
            KeyScope::Community,
            &MediaEncryptionKey::generate(5),
        )
        .unwrap();
        persist_mek(
            &ks,
            "c",
            KeyScope::Community,
            &MediaEncryptionKey::generate(4),
        )
        .unwrap();
        assert_eq!(
            load_latest_mek(&ks, "c", KeyScope::Community)
                .unwrap()
                .generation(),
            5
        );
        assert!(load_mek_generation(&ks, "c", KeyScope::Community, 4).is_some());
    }

    #[test]
    fn survives_reopen() {
        let dir = TempDir::new().unwrap();
        let mek = MediaEncryptionKey::generate(42);
        persist_mek(&keystore(&dir), "c", CH, &mek).unwrap();
        let loaded = load_mek_generation(&keystore(&dir), "c", CH, 42).unwrap();
        assert_eq!(loaded.as_bytes(), mek.as_bytes());
    }

    #[test]
    fn delete_erases_every_generation_of_the_scope_only() {
        let dir = TempDir::new().unwrap();
        let ks = keystore(&dir);
        for generation in 1..=3 {
            persist_mek(
                &ks,
                "c",
                KeyScope::Community,
                &MediaEncryptionKey::generate(generation),
            )
            .unwrap();
        }
        persist_mek(&ks, "c", CH, &MediaEncryptionKey::generate(1)).unwrap();
        delete_scope_meks(&ks, "c", KeyScope::Community);
        for generation in 1..=3 {
            assert!(load_mek_generation(&ks, "c", KeyScope::Community, generation).is_none());
        }
        assert!(load_latest_mek(&ks, "c", KeyScope::Community).is_none());
        assert!(load_latest_mek(&ks, "c", CH).is_some());
    }
}
