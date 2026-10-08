//! Phase 2 — dev-only vault diagnostics command.
//!
//! Surfaces vault row count + last-write timestamp for debugging. Gated
//! behind `cfg(debug_assertions)` so release builds don't expose this.
//! See `/Users/kali/.claude/plans/memoized-dazzling-torvalds.md` § Phase 2.

#[cfg(debug_assertions)]
use serde::Serialize;

#[cfg(debug_assertions)]
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultDiagnostics {
    pub entries: usize,
    /// Vault file mtime in milliseconds since the Unix epoch, or `None`
    /// if the path doesn't exist yet (vault locked) or stat failed.
    pub last_write_ms: Option<u64>,
}

#[cfg(debug_assertions)]
#[tauri::command]
pub async fn vault_diagnostics(
    keystore_handle: tauri::State<'_, crate::keystore::KeystoreHandle>,
) -> Result<VaultDiagnostics, String> {
    let guard = keystore_handle.lock();
    let Some(keystore) = guard.as_ref() else {
        return Err("vault locked — no keystore initialized".to_string());
    };
    let entries = keystore.entry_count().map_err(|e| e.to_string())?;
    let last_write_ms = std::fs::metadata(keystore.vault_path())
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|d| u64::try_from(d.as_millis()).ok());
    Ok(VaultDiagnostics {
        entries,
        last_write_ms,
    })
}
