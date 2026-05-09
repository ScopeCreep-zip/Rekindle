//! Cryptographic key storage — store, load, delete, list.
//!
//! Every secret key (signing key, DhtLog keypairs, prekey private material,
//! governance keypairs, identity seeds) is entry-encrypted before storage.
//! Labels are validated against a strict ASCII alphanumeric + dot + hyphen
//! character set to prevent injection.

pub mod labels;

use rusqlite::{params, OptionalExtension};

use crate::error::{StorageError, StorageResult};
use crate::vault::VaultStore;
use crate::vault::schema::timestamp_secs;

impl VaultStore {
    /// Store a cryptographic key. Overwrites if label already exists.
    pub fn store_key(&self, label: &str, value: &[u8]) -> StorageResult<()> {
        labels::validate(label)?;
        let ct = self.encrypt_entry(value)?;
        let conn = self.conn();
        conn.execute(
            "INSERT INTO keys (label, value, created_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(label) DO UPDATE SET value = ?2",
            params![label, ct, timestamp_secs()],
        )?;
        Ok(())
    }

    /// Load a cryptographic key by label. Returns `None` if not found.
    pub fn load_key(&self, label: &str) -> StorageResult<Option<Vec<u8>>> {
        let conn = self.conn();
        let ct: Option<Vec<u8>> = conn
            .query_row(
                "SELECT value FROM keys WHERE label = ?1",
                params![label],
                |row| row.get(0),
            )
            .optional()?;
        match ct {
            Some(ct) => Ok(Some(self.decrypt_entry(&ct)?)),
            None => Ok(None),
        }
    }

    /// Load a cryptographic key, returning [`StorageError::KeyNotFound`] if absent.
    pub fn require_key(&self, label: &str) -> StorageResult<Vec<u8>> {
        self.load_key(label)?
            .ok_or_else(|| StorageError::KeyNotFound { label: label.to_string() })
    }

    /// Delete a cryptographic key. No-op if the label doesn't exist.
    pub fn delete_key(&self, label: &str) -> StorageResult<()> {
        let conn = self.conn();
        conn.execute("DELETE FROM keys WHERE label = ?1", params![label])?;
        Ok(())
    }

    /// List all key labels. Values are NOT returned.
    pub fn list_key_labels(&self) -> StorageResult<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT label FROM keys ORDER BY label")?;
        let labels: Vec<String> = stmt
            .query_map([], |row| row.get(0))?
            .filter_map(Result::ok)
            .collect();
        Ok(labels)
    }

    /// Count of stored keys.
    pub fn count_keys(&self) -> StorageResult<u64> {
        let conn = self.conn();
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM keys", [], |row| row.get(0))?;
        Ok(u64::try_from(count).unwrap_or(0))
    }
}
