//! Audit log implementation — BLAKE3 keyed hash chain over JSONL.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::{StorageError, StorageResult};
use crate::vault::schema::timestamp_secs;

pub struct AuditLog {
    path: PathBuf,
    key: [u8; 32],
    last_hash: Option<[u8; 32]>,
}

impl AuditLog {
    /// Open or create the audit log. Reads the last entry to seed the chain.
    pub fn open(path: &Path, audit_key: &[u8; 32]) -> StorageResult<Self> {
        let last_hash = if path.exists() {
            let content = std::fs::read_to_string(path).map_err(|e| StorageError::VaultCorrupt {
                reason: format!("audit read: {e}"),
            })?;
            content
                .lines()
                .filter(|l| !l.is_empty())
                .next_back()
                .and_then(|line| {
                    let entry: serde_json::Value = serde_json::from_str(line).ok()?;
                    let h = entry.get("hash")?.as_str()?;
                    let bytes = hex::decode(h).ok()?;
                    <[u8; 32]>::try_from(bytes.as_slice()).ok()
                })
        } else {
            None
        };

        Ok(Self {
            path: path.to_path_buf(),
            key: *audit_key,
            last_hash,
        })
    }

    /// Derive the audit key from the master key.
    pub fn derive_key(master_key: &[u8; 32]) -> [u8; 32] {
        blake3::derive_key("rekindle v1 audit-key", master_key)
    }

    /// Append an entry to the audit log.
    pub fn append(
        &mut self,
        event_type: &str,
        detail: &serde_json::Value,
    ) -> StorageResult<()> {
        let timestamp = timestamp_secs();
        let prev_hex = self
            .last_hash
            .map_or_else(|| "genesis".to_string(), hex::encode);

        let mut hasher = blake3::Hasher::new_keyed(&self.key);
        hasher.update(prev_hex.as_bytes());
        hasher.update(event_type.as_bytes());
        hasher.update(&timestamp.to_le_bytes());
        hasher.update(detail.to_string().as_bytes());
        let hash = hasher.finalize();
        let hash_hex = hex::encode(hash.as_bytes());

        let entry = serde_json::json!({
            "t": timestamp,
            "type": event_type,
            "detail": detail,
            "prev": prev_hex,
            "hash": hash_hex,
        });

        let line = serde_json::to_string(&entry).map_err(|e| StorageError::VaultCorrupt {
            reason: format!("audit serialize: {e}"),
        })?;

        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| StorageError::VaultCorrupt {
                reason: format!("audit open: {e}"),
            })?;
        writeln!(file, "{line}").map_err(|e| StorageError::VaultCorrupt {
            reason: format!("audit write: {e}"),
        })?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600));
        }

        self.last_hash = Some(*hash.as_bytes());
        Ok(())
    }

    /// Verify the entire chain. Returns the number of valid entries.
    pub fn verify_chain(&self) -> StorageResult<u64> {
        let content = std::fs::read_to_string(&self.path).map_err(|e| StorageError::VaultCorrupt {
            reason: format!("audit read: {e}"),
        })?;

        let mut prev = "genesis".to_string();
        let mut count = 0u64;

        for (i, line) in content.lines().enumerate() {
            if line.is_empty() {
                continue;
            }
            let entry: serde_json::Value = serde_json::from_str(line)
                .map_err(|_| StorageError::AuditChainBroken { index: u64::try_from(i).unwrap_or(0) })?;

            let stored_prev = entry.get("prev").and_then(|v| v.as_str()).unwrap_or("");
            if stored_prev != prev {
                return Err(StorageError::AuditChainBroken { index: i as u64 });
            }

            let stored_hash = entry.get("hash").and_then(|v| v.as_str()).unwrap_or("");
            let event_type = entry.get("type").and_then(|v| v.as_str()).unwrap_or("");
            let timestamp = entry.get("t").and_then(serde_json::Value::as_i64).unwrap_or(0);
            let detail = entry
                .get("detail")
                .cloned()
                .unwrap_or(serde_json::Value::Null);

            let mut hasher = blake3::Hasher::new_keyed(&self.key);
            hasher.update(prev.as_bytes());
            hasher.update(event_type.as_bytes());
            hasher.update(&timestamp.to_le_bytes());
            hasher.update(detail.to_string().as_bytes());
            let computed = hex::encode(hasher.finalize().as_bytes());

            if computed != stored_hash {
                return Err(StorageError::AuditChainBroken { index: i as u64 });
            }

            prev = stored_hash.to_string();
            count += 1;
        }

        Ok(count)
    }
}
