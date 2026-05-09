//! Session metadata — `session.json` read/write.
//!
//! Session.json stores non-secret bootstrap metadata: public keys, DHT
//! record keys, community memberships, `dm_peers` mapping. No signing
//! key, no Signal sessions, no keypairs, no MEKs, no message plaintext.
//!
//! Tamper detection via BLAKE3 keyed_hash MAC (SOTA) or HMAC-SHA-256 (FIPS).

pub mod integrity;

use std::path::Path;

use crate::error::{StorageError, StorageResult};

/// Read session.json, verify MAC, return the JSON bytes.
pub fn load(path: &Path, mac_key: &[u8; 32]) -> StorageResult<Vec<u8>> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| StorageError::SessionMetaParse(format!("read: {e}")))?;

    let (json, stored_mac) =
        integrity::split_mac(&content).ok_or(StorageError::SessionMetaIntegrity)?;

    let computed = integrity::compute_mac(mac_key, json.as_bytes());
    if computed != stored_mac {
        return Err(StorageError::SessionMetaIntegrity);
    }

    Ok(json.as_bytes().to_vec())
}

/// Write session.json with appended MAC. Atomic via tempfile + rename.
pub fn save(path: &Path, mac_key: &[u8; 32], json: &[u8]) -> StorageResult<()> {
    let mac = integrity::compute_mac(mac_key, json);
    let json_str =
        std::str::from_utf8(json).map_err(|e| StorageError::SessionMetaParse(format!("{e}")))?;

    let content = format!("{json_str}\n---MAC---\n{mac}");

    let tmp = path.with_extension("tmp");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| StorageError::VaultCreationFailed {
            reason: format!("mkdir: {e}"),
        })?;
    }
    std::fs::write(&tmp, &content).map_err(|e| StorageError::VaultCreationFailed {
        reason: format!("write: {e}"),
    })?;
    std::fs::rename(&tmp, path).map_err(|e| StorageError::VaultCreationFailed {
        reason: format!("rename: {e}"),
    })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// Derive the session MAC key from the master key.
pub fn derive_mac_key(master_key: &[u8; 32]) -> [u8; 32] {
    blake3::derive_key("rekindle v1 session-mac", master_key)
}
