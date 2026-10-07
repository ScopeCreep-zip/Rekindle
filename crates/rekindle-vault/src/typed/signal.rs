//! Signal store persistence (B7/D4, P0.5).
//!
//! Architecture §11 — Signal sessions, prekeys and trusted peer identities must persist across
//! restart so a corrupted-on-disk session is recoverable, not the default
//! state. The previous Memory*Store implementations lost everything on app
//! exit, forcing every friend to re-handshake on every launch. For a
//! vulnerable user this is a social-engineering opportunity: an attacker
//! who can prompt a re-handshake can substitute their own keys.
//!
//! All helpers mirror the persist_mek / load_mek pattern:
//! - take the open `&VaultStore` (the caller holds the keystore lock)
//! - address each entry with a typed [`VaultKey`] (the storage security
//!   boundary) so key names can't collide or be mistyped
//!
//! Indices: the vault has no list-keys API, so for the multi-entry stores
//! (sessions, prekeys) we maintain a separate "index" record holding the
//! current keys, in insertion order. A failed index read or write is an
//! error like any other vault failure.

use zeroize::Zeroizing;

use crate::{VaultKey, VaultStore};

/// Persist a per-peer trusted-identity entry (TOFU).
pub fn persist_trusted_identity(
    vault: &VaultStore,
    peer_address: &str,
    identity_key: &[u8],
) -> Result<(), String> {
    vault
        .put(
            &VaultKey::SignalTrusted {
                peer: peer_address.to_string(),
            },
            identity_key,
        )
        .map_err(|e| format!("persist trusted identity: {e}"))
}

/// Load the trusted identity for a peer (None if no prior interaction).
pub fn load_trusted_identity(vault: &VaultStore, peer_address: &str) -> Option<Vec<u8>> {
    vault
        .get(&VaultKey::SignalTrusted {
            peer: peer_address.to_string(),
        })
        .ok()?
        .map(|key| key.to_vec())
}

/// Persist a Signal session for a peer, updating the session index so the
/// store can list known peers after restart.
pub fn persist_signal_session(
    vault: &VaultStore,
    peer_address: &str,
    session_data: &[u8],
) -> Result<(), String> {
    vault
        .put(
            &VaultKey::SignalSession {
                peer: peer_address.to_string(),
            },
            session_data,
        )
        .map_err(|e| format!("persist signal session: {e}"))?;
    add_to_string_index(vault, &VaultKey::SignalSessionIndex, peer_address)
}

/// Load a Signal session for a peer (None if no prior session).
pub fn load_signal_session(vault: &VaultStore, peer_address: &str) -> Option<Zeroizing<Vec<u8>>> {
    vault
        .get(&VaultKey::SignalSession {
            peer: peer_address.to_string(),
        })
        .ok()?
}

/// Delete a Signal session and remove it from the index.
pub fn delete_signal_session(vault: &VaultStore, peer_address: &str) -> Result<(), String> {
    vault
        .delete(&VaultKey::SignalSession {
            peer: peer_address.to_string(),
        })
        .map_err(|e| format!("delete signal session: {e}"))?;
    remove_from_string_index(vault, &VaultKey::SignalSessionIndex, peer_address)
}

/// List all peers with persisted Signal sessions (used at login to populate
/// the in-memory cache).
pub fn list_signal_sessions(vault: &VaultStore) -> Result<Vec<String>, String> {
    load_string_index(vault, &VaultKey::SignalSessionIndex)
}

/// Persist a one-time prekey, appending it to the prekey index.
pub fn persist_signal_prekey(
    vault: &VaultStore,
    prekey_id: u32,
    key_data: &[u8],
) -> Result<(), String> {
    vault
        .put(&VaultKey::SignalPrekey { id: prekey_id }, key_data)
        .map_err(|e| format!("persist signal prekey: {e}"))?;
    add_to_string_index(vault, &VaultKey::SignalPrekeyIndex, &prekey_id.to_string())
}

/// Load a one-time prekey by id (`Ok(None)` if missing or consumed).
pub fn load_signal_prekey(
    vault: &VaultStore,
    prekey_id: u32,
) -> Result<Option<Zeroizing<Vec<u8>>>, String> {
    vault
        .get(&VaultKey::SignalPrekey { id: prekey_id })
        .map_err(|e| format!("load signal prekey {prekey_id}: {e}"))
}

/// Delete a consumed one-time prekey and remove it from the index.
pub fn delete_signal_prekey(vault: &VaultStore, prekey_id: u32) -> Result<(), String> {
    vault
        .delete(&VaultKey::SignalPrekey { id: prekey_id })
        .map_err(|e| format!("delete signal prekey {prekey_id}: {e}"))?;
    remove_from_string_index(vault, &VaultKey::SignalPrekeyIndex, &prekey_id.to_string())
}

/// Persisted one-time prekey ids, oldest first.
pub fn list_signal_prekey_ids(vault: &VaultStore) -> Result<Vec<u32>, String> {
    load_u32_index(vault, &VaultKey::SignalPrekeyIndex)
}

fn pq_vault_key(prekey_id: u32, last_resort: bool) -> VaultKey {
    if last_resort {
        VaultKey::SignalPqLastResort { id: prekey_id }
    } else {
        VaultKey::SignalPqOneTime { id: prekey_id }
    }
}

/// Persist an ML-KEM-768 secret (PQXDH last-resort or one-time). One-time
/// ids are appended to the PQ one-time index.
pub fn persist_signal_pq_secret(
    vault: &VaultStore,
    prekey_id: u32,
    last_resort: bool,
    key_data: &[u8],
) -> Result<(), String> {
    vault
        .put(&pq_vault_key(prekey_id, last_resort), key_data)
        .map_err(|e| format!("persist pq secret: {e}"))?;
    if last_resort {
        return Ok(());
    }
    add_to_string_index(
        vault,
        &VaultKey::SignalPqOneTimeIndex,
        &prekey_id.to_string(),
    )
}

/// Load an ML-KEM-768 secret by id and kind (`Ok(None)` if missing).
pub fn load_signal_pq_secret(
    vault: &VaultStore,
    prekey_id: u32,
    last_resort: bool,
) -> Result<Option<Zeroizing<Vec<u8>>>, String> {
    vault
        .get(&pq_vault_key(prekey_id, last_resort))
        .map_err(|e| format!("load pq secret {prekey_id}: {e}"))
}

/// Delete a consumed ML-KEM-768 one-time secret and remove it from the index.
pub fn delete_signal_pq_one_time(vault: &VaultStore, prekey_id: u32) -> Result<(), String> {
    vault
        .delete(&pq_vault_key(prekey_id, false))
        .map_err(|e| format!("delete pq one-time secret {prekey_id}: {e}"))?;
    remove_from_string_index(
        vault,
        &VaultKey::SignalPqOneTimeIndex,
        &prekey_id.to_string(),
    )
}

/// Persisted PQ one-time prekey ids, oldest first.
pub fn list_signal_pq_one_time_ids(vault: &VaultStore) -> Result<Vec<u32>, String> {
    load_u32_index(vault, &VaultKey::SignalPqOneTimeIndex)
}

/// Persist a signed prekey by id.
pub fn persist_signal_signed_prekey(
    vault: &VaultStore,
    signed_prekey_id: u32,
    key_data: &[u8],
) -> Result<(), String> {
    vault
        .put(
            &VaultKey::SignalSignedPrekey {
                id: signed_prekey_id,
            },
            key_data,
        )
        .map_err(|e| format!("persist signed prekey: {e}"))
}

/// Load a signed prekey by id (`Ok(None)` if missing).
pub fn load_signal_signed_prekey(
    vault: &VaultStore,
    signed_prekey_id: u32,
) -> Result<Option<Zeroizing<Vec<u8>>>, String> {
    vault
        .get(&VaultKey::SignalSignedPrekey {
            id: signed_prekey_id,
        })
        .map_err(|e| format!("load signed prekey {signed_prekey_id}: {e}"))
}

// ─── String-index helpers (the vault has no list-keys API) ───────────────────

fn load_string_index(vault: &VaultStore, index_key: &VaultKey) -> Result<Vec<String>, String> {
    let Some(blob) = vault
        .get(index_key)
        .map_err(|e| format!("load index: {e}"))?
    else {
        return Ok(Vec::new());
    };
    serde_json::from_slice::<Vec<String>>(&blob).map_err(|e| format!("parse index: {e}"))
}

fn load_u32_index(vault: &VaultStore, index_key: &VaultKey) -> Result<Vec<u32>, String> {
    load_string_index(vault, index_key)?
        .iter()
        .map(|s| {
            s.parse::<u32>()
                .map_err(|e| format!("index entry {s:?}: {e}"))
        })
        .collect()
}

fn save_string_index(
    vault: &VaultStore,
    index_key: &VaultKey,
    entries: &[String],
) -> Result<(), String> {
    let blob = serde_json::to_vec(entries).map_err(|e| format!("serialize index: {e}"))?;
    vault
        .put(index_key, &blob)
        .map_err(|e| format!("store index: {e}"))
}

fn add_to_string_index(
    vault: &VaultStore,
    index_key: &VaultKey,
    entry: &str,
) -> Result<(), String> {
    let mut entries = load_string_index(vault, index_key)?;
    if !entries.iter().any(|e| e == entry) {
        entries.push(entry.to_string());
        save_string_index(vault, index_key, &entries)?;
    }
    Ok(())
}

fn remove_from_string_index(
    vault: &VaultStore,
    index_key: &VaultKey,
    entry: &str,
) -> Result<(), String> {
    let mut entries = load_string_index(vault, index_key)?;
    let original_len = entries.len();
    entries.retain(|e| e != entry);
    if entries.len() != original_len {
        save_string_index(vault, index_key, &entries)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn vault(dir: &TempDir) -> VaultStore {
        VaultStore::open(&dir.path().join("rekindle.vault"), "pp").unwrap()
    }

    #[test]
    fn sessions_and_prekeys_are_listed_until_deleted() {
        let dir = TempDir::new().unwrap();
        let v = vault(&dir);
        persist_signal_session(&v, "bob", b"s1").unwrap();
        persist_signal_session(&v, "bob", b"s2").unwrap();
        persist_signal_session(&v, "carol", b"s3").unwrap();
        assert_eq!(list_signal_sessions(&v).unwrap(), ["bob", "carol"]);
        assert_eq!(load_signal_session(&v, "bob").unwrap().as_slice(), b"s2");
        delete_signal_session(&v, "bob").unwrap();
        assert_eq!(list_signal_sessions(&v).unwrap(), ["carol"]);
        assert!(load_signal_session(&v, "bob").is_none());

        persist_signal_prekey(&v, 7, b"k7").unwrap();
        persist_signal_pq_secret(&v, 9, false, b"pq9").unwrap();
        persist_signal_pq_secret(&v, 1, true, b"lr").unwrap();
        assert_eq!(list_signal_prekey_ids(&v).unwrap(), [7]);
        assert_eq!(list_signal_pq_one_time_ids(&v).unwrap(), [9]);
        delete_signal_prekey(&v, 7).unwrap();
        delete_signal_pq_one_time(&v, 9).unwrap();
        assert!(list_signal_prekey_ids(&v).unwrap().is_empty());
        assert!(list_signal_pq_one_time_ids(&v).unwrap().is_empty());
        assert_eq!(
            load_signal_pq_secret(&v, 1, true)
                .unwrap()
                .unwrap()
                .as_slice(),
            b"lr"
        );
    }
}
