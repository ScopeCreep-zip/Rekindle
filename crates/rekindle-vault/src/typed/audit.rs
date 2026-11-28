//! The audit chain's MAC key and tail anchor.
//!
//! Stored under a dedicated `"audit"` namespace so it can't collide with
//! Signal entries. The MAC key is generated on first
//! `load_or_create_audit_mac_key` call and reused on every subsequent unlock;
//! the tail anchor is an out-of-band proof against SQLite-side truncation.

use crate::{VaultKey, VaultStore};

/// Load the audit MAC key, generating + persisting a fresh one on first call.
/// Returns 32 bytes suitable for `AuditChain::open`.
///
/// # Errors
/// Returns `String` on vault I/O failure or RNG failure.
pub fn load_or_create_audit_mac_key(vault: &VaultStore) -> Result<[u8; 32], String> {
    use rand::RngCore;

    // Distinguish three states explicitly:
    //   Ok(Some(32B)) → reuse existing key (idempotent).
    //   Ok(Some(wrong-length)) → corrupt; regenerate (existing chain becomes
    //                            unverifiable — that's the tamper signal).
    //   Ok(None) → no key yet; generate one.
    //   Err(_) → transient I/O failure; refuse to overwrite a potentially
    //            recoverable key, propagate the error so the caller can
    //            either retry or leave the chain disabled.
    match vault.get(&VaultKey::AuditMacKey) {
        Ok(Some(existing)) => {
            if existing.len() == 32 {
                let mut out = [0u8; 32];
                out.copy_from_slice(&existing);
                return Ok(out);
            }
            tracing::warn!(
                len = existing.len(),
                "audit MAC key has wrong length — regenerating (existing chain will fail verify)",
            );
        }
        Ok(None) => {} // fall through to generate
        Err(e) => {
            return Err(format!(
                "load audit mac key failed (refusing to overwrite — chain stays disabled until \
                 the vault is readable): {e}"
            ));
        }
    }
    let mut key_bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut key_bytes);
    vault
        .put(&VaultKey::AuditMacKey, &key_bytes)
        .map_err(|e| format!("persist audit mac key: {e}"))?;
    Ok(key_bytes)
}

/// Persist the current chain tail (cursor + mac) to the vault. Used as an
/// out-of-band anchor for detecting SQLite-side tail truncation: an attacker
/// can drop trailing `audit_entries` rows, but they cannot forge a matching
/// vault update without the vault passphrase.
///
/// Wire format: 8 bytes LE cursor || 32 bytes mac = 40 bytes total.
///
/// # Errors
/// Returns `String` on vault I/O failure.
pub fn persist_audit_tail(vault: &VaultStore, cursor: u64, mac: &[u8; 32]) -> Result<(), String> {
    let mut payload = Vec::with_capacity(40);
    payload.extend_from_slice(&cursor.to_le_bytes());
    payload.extend_from_slice(mac);
    vault
        .put(&VaultKey::AuditTail, &payload)
        .map_err(|e| format!("persist audit tail: {e}"))
}

/// Load the persisted tail anchor written by `persist_audit_tail`, or
/// `None` if no anchor has been written yet (fresh identity).
#[must_use]
pub fn load_audit_tail(vault: &VaultStore) -> Option<(u64, [u8; 32])> {
    let bytes = vault.get(&VaultKey::AuditTail).ok()??;
    if bytes.len() != 40 {
        tracing::warn!(
            len = bytes.len(),
            "audit tail anchor has wrong length — ignoring (chain will be treated as fresh)",
        );
        return None;
    }
    let mut cursor_bytes = [0u8; 8];
    cursor_bytes.copy_from_slice(&bytes[..8]);
    let cursor = u64::from_le_bytes(cursor_bytes);
    let mut mac = [0u8; 32];
    mac.copy_from_slice(&bytes[8..40]);
    Some((cursor, mac))
}

#[cfg(test)]
#[path = "audit_tests.rs"]
mod tests;
