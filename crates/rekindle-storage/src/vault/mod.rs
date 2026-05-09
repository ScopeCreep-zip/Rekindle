//! Vault lifecycle — create, open, close.
//!
//! [`VaultStore`] is the single entry point for all encrypted persistent
//! storage. One instance per identity. Opened on daemon unlock. Closed on
//! lock/shutdown. All methods are synchronous.

pub mod entry_crypto;
pub mod pragma;
pub mod schema;

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::Connection;
use zeroize::Zeroizing;

use crate::error::{StorageError, StorageResult};

/// Encrypted persistent storage backed by SQLCipher with double encryption.
///
/// Layer 1: SQLCipher page-level AES-256-CBC (`PRAGMA key = vault_key`).
/// Layer 2: Per-entry AES-256-GCM (`entry_key`, derived independently).
///
/// The `Mutex<Connection>` serializes all database access. At 100K agents
/// the node crate's per-session `tokio::sync::Mutex` ensures only one
/// ratchet step per session runs at a time; the vault sees serialized
/// writes bounded at ~1000 concurrent (1% of 100K ratcheting simultaneously).
pub struct VaultStore {
    conn: Mutex<Connection>,
    entry_key: Zeroizing<[u8; 32]>,
    #[allow(dead_code)]
    db_path: PathBuf,
}

impl VaultStore {
    /// Create a new vault. Called exactly once during `rekindle init`.
    pub fn create(db_path: &Path, master_key: &[u8; 32]) -> StorageResult<Self> {
        if db_path.exists() {
            return Err(StorageError::VaultAlreadyExists);
        }
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| StorageError::VaultCreationFailed {
                reason: format!("directory: {e}"),
            })?;
        }

        let vault_key = derive_vault_key(master_key);
        let conn = Connection::open(db_path).map_err(|e| StorageError::VaultCreationFailed {
            reason: format!("sqlite open: {e}"),
        })?;

        pragma::configure(&conn, &vault_key)?;
        schema::create_all(&conn)?;

        let entry_key = derive_entry_key(master_key);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(db_path, std::fs::Permissions::from_mode(0o600));
        }

        Ok(Self {
            conn: Mutex::new(conn),
            entry_key,
            db_path: db_path.to_path_buf(),
        })
    }

    /// Open an existing vault. Called on every daemon unlock.
    pub fn open(db_path: &Path, master_key: &[u8; 32]) -> StorageResult<Self> {
        if !db_path.exists() {
            return Err(StorageError::VaultOpenFailed {
                reason: "vault.db does not exist".into(),
            });
        }

        let vault_key = derive_vault_key(master_key);
        let conn = Connection::open(db_path).map_err(|e| StorageError::VaultOpenFailed {
            reason: format!("sqlite open: {e}"),
        })?;

        pragma::configure(&conn, &vault_key)?;

        // Verify the key is correct — wrong key yields "not a database"
        conn.execute_batch("SELECT count(*) FROM sqlite_master;")
            .map_err(|e| StorageError::VaultOpenFailed {
                reason: format!("key verification failed: {e}"),
            })?;

        schema::migrate(&conn)?;

        let entry_key = derive_entry_key(master_key);

        Ok(Self {
            conn: Mutex::new(conn),
            entry_key,
            db_path: db_path.to_path_buf(),
        })
    }

    /// Close the vault. Zeroizes all key material. Clears SQLCipher buffers.
    /// Consumes self — cannot be reused after close.
    pub fn close(mut self) {
        if let Ok(conn) = self.conn.get_mut() {
            let _ = conn.execute_batch("PRAGMA rekey = '';");
        }
        // entry_key zeroed by Zeroizing<[u8;32]> Drop
        // Connection closed by Mutex<Connection> Drop
    }

    /// Encrypt a value with per-entry AES-256-GCM.
    pub(crate) fn encrypt_entry(&self, plaintext: &[u8]) -> StorageResult<Vec<u8>> {
        entry_crypto::encrypt(&self.entry_key, plaintext)
    }

    /// Decrypt a value with per-entry AES-256-GCM.
    pub(crate) fn decrypt_entry(&self, wire: &[u8]) -> StorageResult<Vec<u8>> {
        entry_crypto::decrypt(&self.entry_key, wire)
    }

    /// Acquire the database connection.
    pub(crate) fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().expect("vault mutex poisoned — unrecoverable")
    }
}

impl Drop for VaultStore {
    fn drop(&mut self) {
        if let Ok(conn) = self.conn.get_mut() {
            let _ = conn.execute_batch("PRAGMA rekey = '';");
        }
    }
}

// ── Key derivation ─────────────────────────────────────────────────

fn derive_vault_key(master_key: &[u8; 32]) -> Zeroizing<[u8; 32]> {
    let derived = blake3::derive_key("rekindle v1 vault-key", master_key);
    Zeroizing::new(derived)
}

fn derive_entry_key(master_key: &[u8; 32]) -> Zeroizing<[u8; 32]> {
    let derived = blake3::derive_key("rekindle v1 entry-key", master_key);
    Zeroizing::new(derived)
}
