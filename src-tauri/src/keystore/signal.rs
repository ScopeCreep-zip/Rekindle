//! Signal store persistence (B7/D4, P0.5).
//!
//! Architecture §11 — Signal sessions/prekeys/identity must persist across
//! restart so a corrupted-on-disk session is recoverable, not the default
//! state. The previous Memory*Store implementations lost everything on app
//! exit, forcing every friend to re-handshake on every launch. For a
//! vulnerable user this is a social-engineering opportunity: an attacker
//! who can prompt a re-handshake can substitute their own keys.
//!
//! All helpers mirror the persist_mek / load_mek pattern:
//! - take a `&StrongholdKeystore` handle (already locked by caller)
//! - address each entry with a typed [`VaultKey`] (the storage security
//!   boundary) so key names can't collide or be mistyped
//!
//! Indices: the vault has no list-keys API, so for the multi-entry stores
//! (sessions, prekeys) we maintain a separate "index" record holding the
//! current keys. Index updates are best-effort (log on fail); the per-entry
//! persist is the authoritative write.

use super::StrongholdKeystore;
use rekindle_vault::VaultKey;

/// Persist Signal identity key pair + registration ID. Loaded once at login;
/// the in-memory store mirrors it for fast access.
pub fn persist_signal_identity(
    keystore: &StrongholdKeystore,
    identity_private: &[u8],
    identity_public: &[u8],
    registration_id: u32,
) -> Result<(), String> {
    let mut blob = Vec::with_capacity(identity_private.len() + identity_public.len() + 8);
    blob.extend_from_slice(
        &u32::try_from(identity_private.len())
            .unwrap_or(0)
            .to_le_bytes(),
    );
    blob.extend_from_slice(identity_private);
    blob.extend_from_slice(
        &u32::try_from(identity_public.len())
            .unwrap_or(0)
            .to_le_bytes(),
    );
    blob.extend_from_slice(identity_public);
    keystore
        .vault_put(&VaultKey::SignalIdentity, &blob)
        .map_err(|e| format!("persist signal identity: {e}"))?;
    keystore
        .vault_put(
            &VaultKey::SignalRegistrationId,
            &registration_id.to_le_bytes(),
        )
        .map_err(|e| format!("persist signal registration_id: {e}"))
}

/// Load the persisted Signal identity. Returns None if never persisted.
pub fn load_signal_identity(keystore: &StrongholdKeystore) -> Option<(Vec<u8>, Vec<u8>, u32)> {
    let blob = keystore.vault_get(&VaultKey::SignalIdentity).ok()??;
    if blob.len() < 8 {
        return None;
    }
    let priv_len = u32::from_le_bytes(blob[0..4].try_into().ok()?) as usize;
    if blob.len() < 4 + priv_len + 4 {
        return None;
    }
    let private = blob[4..4 + priv_len].to_vec();
    let pub_len_offset = 4 + priv_len;
    let pub_len =
        u32::from_le_bytes(blob[pub_len_offset..pub_len_offset + 4].try_into().ok()?) as usize;
    if blob.len() < pub_len_offset + 4 + pub_len {
        return None;
    }
    let public = blob[pub_len_offset + 4..pub_len_offset + 4 + pub_len].to_vec();
    let reg_bytes = keystore.vault_get(&VaultKey::SignalRegistrationId).ok()??;
    let registration_id = u32::from_le_bytes(reg_bytes.as_slice().try_into().ok()?);
    Some((private, public, registration_id))
}

/// Persist a per-peer trusted-identity entry (TOFU).
pub fn persist_trusted_identity(
    keystore: &StrongholdKeystore,
    peer_address: &str,
    identity_key: &[u8],
) -> Result<(), String> {
    keystore
        .vault_put(
            &VaultKey::SignalTrusted {
                peer: peer_address.to_string(),
            },
            identity_key,
        )
        .map_err(|e| format!("persist trusted identity: {e}"))
}

/// Load the trusted identity for a peer (None if no prior interaction).
pub fn load_trusted_identity(keystore: &StrongholdKeystore, peer_address: &str) -> Option<Vec<u8>> {
    keystore
        .vault_get(&VaultKey::SignalTrusted {
            peer: peer_address.to_string(),
        })
        .ok()?
}

/// Persist a Signal session for a peer, updating the session index so the
/// store can list known peers after restart.
pub fn persist_signal_session(
    keystore: &StrongholdKeystore,
    peer_address: &str,
    session_data: &[u8],
) -> Result<(), String> {
    keystore
        .vault_put(
            &VaultKey::SignalSession {
                peer: peer_address.to_string(),
            },
            session_data,
        )
        .map_err(|e| format!("persist signal session: {e}"))?;
    add_to_string_index(keystore, &VaultKey::SignalSessionIndex, peer_address)
}

/// Load a Signal session for a peer (None if no prior session).
pub fn load_signal_session(keystore: &StrongholdKeystore, peer_address: &str) -> Option<Vec<u8>> {
    keystore
        .vault_get(&VaultKey::SignalSession {
            peer: peer_address.to_string(),
        })
        .ok()?
}

/// Delete a Signal session and remove it from the index.
pub fn delete_signal_session(keystore: &StrongholdKeystore, peer_address: &str) {
    if let Err(e) = keystore.vault_delete(&VaultKey::SignalSession {
        peer: peer_address.to_string(),
    }) {
        tracing::warn!(peer = %peer_address, error = %e, "delete signal session failed");
    }
    let _ = remove_from_string_index(keystore, &VaultKey::SignalSessionIndex, peer_address);
}

/// List all peers with persisted Signal sessions (used at login to populate
/// the in-memory cache).
pub fn list_signal_sessions(keystore: &StrongholdKeystore) -> Vec<String> {
    load_string_index(keystore, &VaultKey::SignalSessionIndex)
}

/// Persist a one-time prekey, updating the prekey index.
pub fn persist_signal_prekey(
    keystore: &StrongholdKeystore,
    prekey_id: u32,
    key_data: &[u8],
) -> Result<(), String> {
    keystore
        .vault_put(&VaultKey::SignalPrekey { id: prekey_id }, key_data)
        .map_err(|e| format!("persist signal prekey: {e}"))?;
    add_to_string_index(
        keystore,
        &VaultKey::SignalPrekeyIndex,
        &prekey_id.to_string(),
    )
}

/// Load a one-time prekey by id (None if missing or already consumed).
pub fn load_signal_prekey(keystore: &StrongholdKeystore, prekey_id: u32) -> Option<Vec<u8>> {
    keystore
        .vault_get(&VaultKey::SignalPrekey { id: prekey_id })
        .ok()?
}

/// Delete a consumed one-time prekey and remove from index.
pub fn delete_signal_prekey(keystore: &StrongholdKeystore, prekey_id: u32) {
    if let Err(e) = keystore.vault_delete(&VaultKey::SignalPrekey { id: prekey_id }) {
        tracing::warn!(prekey_id, error = %e, "delete signal prekey failed");
    }
    let _ = remove_from_string_index(
        keystore,
        &VaultKey::SignalPrekeyIndex,
        &prekey_id.to_string(),
    );
}

/// List all currently-persisted one-time prekey IDs.
pub fn list_signal_prekey_ids(keystore: &StrongholdKeystore) -> Vec<u32> {
    load_string_index(keystore, &VaultKey::SignalPrekeyIndex)
        .into_iter()
        .filter_map(|s| s.parse::<u32>().ok())
        .collect()
}

fn pq_vault_key(prekey_id: u32, last_resort: bool) -> VaultKey {
    if last_resort {
        VaultKey::SignalPqLastResort { id: prekey_id }
    } else {
        VaultKey::SignalPqOneTime { id: prekey_id }
    }
}

/// Persist an ML-KEM-768 secret (PQXDH last-resort or one-time).
pub fn persist_signal_pq_secret(
    keystore: &StrongholdKeystore,
    prekey_id: u32,
    last_resort: bool,
    key_data: &[u8],
) -> Result<(), String> {
    keystore
        .vault_put(&pq_vault_key(prekey_id, last_resort), key_data)
        .map_err(|e| format!("persist pq secret: {e}"))
}

/// Load an ML-KEM-768 secret by id and kind.
pub fn load_signal_pq_secret(
    keystore: &StrongholdKeystore,
    prekey_id: u32,
    last_resort: bool,
) -> Option<Vec<u8>> {
    keystore
        .vault_get(&pq_vault_key(prekey_id, last_resort))
        .ok()?
}

/// Delete a consumed ML-KEM-768 secret.
pub fn delete_signal_pq_secret(keystore: &StrongholdKeystore, prekey_id: u32, last_resort: bool) {
    if let Err(e) = keystore.vault_delete(&pq_vault_key(prekey_id, last_resort)) {
        tracing::warn!(prekey_id, last_resort, error = %e, "delete pq secret failed");
    }
}

/// Persist a signed prekey by id.
pub fn persist_signal_signed_prekey(
    keystore: &StrongholdKeystore,
    signed_prekey_id: u32,
    key_data: &[u8],
) -> Result<(), String> {
    keystore
        .vault_put(
            &VaultKey::SignalSignedPrekey {
                id: signed_prekey_id,
            },
            key_data,
        )
        .map_err(|e| format!("persist signed prekey: {e}"))
}

/// Load a signed prekey by id (None if missing).
pub fn load_signal_signed_prekey(
    keystore: &StrongholdKeystore,
    signed_prekey_id: u32,
) -> Option<Vec<u8>> {
    keystore
        .vault_get(&VaultKey::SignalSignedPrekey {
            id: signed_prekey_id,
        })
        .ok()?
}

// ─── String-index helpers (the vault has no list-keys API) ───────────────────

fn load_string_index(keystore: &StrongholdKeystore, index_key: &VaultKey) -> Vec<String> {
    let Ok(Some(blob)) = keystore.vault_get(index_key) else {
        return Vec::new();
    };
    serde_json::from_slice::<Vec<String>>(&blob).unwrap_or_default()
}

fn save_string_index(
    keystore: &StrongholdKeystore,
    index_key: &VaultKey,
    entries: &[String],
) -> Result<(), String> {
    let blob = serde_json::to_vec(entries).map_err(|e| format!("serialize index: {e}"))?;
    keystore
        .vault_put(index_key, &blob)
        .map_err(|e| format!("store index: {e}"))
}

fn add_to_string_index(
    keystore: &StrongholdKeystore,
    index_key: &VaultKey,
    entry: &str,
) -> Result<(), String> {
    let mut entries = load_string_index(keystore, index_key);
    if !entries.iter().any(|e| e == entry) {
        entries.push(entry.to_string());
        save_string_index(keystore, index_key, &entries)?;
    }
    Ok(())
}

fn remove_from_string_index(
    keystore: &StrongholdKeystore,
    index_key: &VaultKey,
    entry: &str,
) -> Result<(), String> {
    let mut entries = load_string_index(keystore, index_key);
    let original_len = entries.len();
    entries.retain(|e| e != entry);
    if entries.len() != original_len {
        save_string_index(keystore, index_key, &entries)?;
    }
    Ok(())
}
