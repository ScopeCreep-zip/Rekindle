//! The desktop's vault handle: one `VaultStore` per identity, opened at
//! login. The typed helpers for each kind of secret are
//! `rekindle_vault::typed` (plan C5.4); they take the store this wraps.

pub use rekindle_vault::typed::audit::{
    load_audit_tail, load_or_create_audit_mac_key, persist_audit_tail,
};
pub use rekindle_vault::typed::community_keys::{
    delete_registry_keypair, delete_slot_keypair, delete_slot_seed, load_registry_keypair,
    load_slot_keypair, load_slot_seed, persist_registry_keypair, persist_slot_keypair,
    persist_slot_seed,
};
pub use rekindle_vault::typed::mek::{
    delete_scope_meks, load_latest_mek, load_mek_generation, persist_mek,
};
pub use rekindle_vault::typed::signal::{
    delete_signal_pq_one_time, delete_signal_prekey, delete_signal_session,
    list_signal_pq_one_time_ids, list_signal_prekey_ids, list_signal_sessions,
    load_signal_pq_secret, load_signal_prekey, load_signal_session, load_signal_signed_prekey,
    load_trusted_identity, persist_signal_pq_secret, persist_signal_prekey, persist_signal_session,
    persist_signal_signed_prekey, persist_trusted_identity,
};

use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;

use rekindle_crypto::CryptoError;
use rekindle_vault::{VaultKey, VaultStore};

/// VaultStore-backed keystore.
///
/// On disk: each identity has `{vault_dir}/{public_key_hex}.vault`
/// (SQLCipher AES-256-CBC at page level) plus a sidecar
/// `{public_key_hex}.vault.salt` (32-byte plaintext random salt — needs
/// to be readable before SQLCipher decryption). Each row in the entries
/// table is additionally sealed with AES-256-GCM under a per-entry key
/// derived from the same passphrase.
pub struct StrongholdKeystore {
    vault: VaultStore,
}

/// The typed helpers in `rekindle_vault::typed` take the store itself.
impl std::ops::Deref for StrongholdKeystore {
    type Target = VaultStore;

    fn deref(&self) -> &VaultStore {
        &self.vault
    }
}

/// Thread-safe handle to the keystore, stored in Tauri managed state.
pub type KeystoreHandle = Arc<Mutex<Option<StrongholdKeystore>>>;

/// Create a new empty keystore handle (unlocked later with passphrase).
pub fn new_handle() -> KeystoreHandle {
    Arc::new(Mutex::new(None))
}

impl StrongholdKeystore {
    /// Create the per-identity vault `{vault_dir}/{public_key_hex}.vault`
    /// (and its `.hdr`) for a new identity.
    pub fn create_for_identity(
        vault_dir: &Path,
        public_key_hex: &str,
        passphrase: &str,
    ) -> Result<Self, CryptoError> {
        let path = vault_dir.join(format!("{public_key_hex}.vault"));
        VaultStore::create(&path, passphrase)
            .map(|(vault, _app_db_key)| Self { vault })
            .map_err(map_vault_error)
    }

    /// Open an existing identity's vault. Nothing is created.
    pub fn initialize_for_identity(
        vault_dir: &Path,
        public_key_hex: &str,
        passphrase: &str,
    ) -> Result<Self, CryptoError> {
        let path = vault_dir.join(format!("{public_key_hex}.vault"));
        VaultStore::open(&path, passphrase)
            .map(|(vault, _app_db_key)| Self { vault })
            .map_err(map_vault_error)
    }

    /// Delete the on-disk vault for a specific identity (`{pk}.vault` and
    /// its `.hdr`). Missing files are tolerated.
    pub fn delete_snapshot(vault_dir: &Path, public_key_hex: &str) -> Result<(), std::io::Error> {
        let path = vault_dir.join(format!("{public_key_hex}.vault"));
        VaultStore::delete_files(&path).map_err(|e| match e {
            rekindle_vault::VaultError::Io(io) => io,
            other => std::io::Error::other(other.to_string()),
        })
    }

    /// Number of entries in the vault — exposed for the dev-only
    /// `vault_diagnostics` Tauri command.
    pub fn entry_count(&self) -> Result<usize, CryptoError> {
        self.vault
            .entry_count()
            .map_err(|e| CryptoError::storage(format!("vault count: {e}")))
    }

    /// On-disk path of this vault — exposed for diagnostics. Used to read
    /// the file's mtime for the `last_write_ms` field of vault_diagnostics.
    pub fn vault_path(&self) -> &std::path::Path {
        self.vault.path()
    }

    /// Seal and store `data` under the typed `vault_key` (the typed
    /// [`VaultKey`] is the storage security boundary: there is no stringly
    /// `(namespace, key)` entry point to misuse).
    pub(crate) fn vault_put(&self, vault_key: &VaultKey, data: &[u8]) -> Result<(), CryptoError> {
        self.vault
            .put(vault_key, data)
            .map_err(|e| CryptoError::storage(format!("vault put: {e}")))
    }

    /// Load and decrypt the entry addressed by `vault_key`, or `None`.
    pub(crate) fn vault_get(&self, vault_key: &VaultKey) -> Result<Option<Vec<u8>>, CryptoError> {
        self.vault
            .get(vault_key)
            .map(|opt| opt.map(|z| z.to_vec()))
            .map_err(|e| CryptoError::storage(format!("vault get: {e}")))
    }
}

/// The test vault for identity "t" in `dir`: created on first use and
/// reopened after, as the tests reopen a vault to simulate a restart.
#[cfg(test)]
pub(crate) fn test_keystore(dir: &Path, pass: &str) -> Result<StrongholdKeystore, CryptoError> {
    if dir.join("t.vault").exists() {
        StrongholdKeystore::initialize_for_identity(dir, "t", pass)
    } else {
        StrongholdKeystore::create_for_identity(dir, "t", pass)
    }
}

fn map_vault_error(e: rekindle_vault::VaultError) -> CryptoError {
    match e {
        rekindle_vault::VaultError::WrongPassphrase => CryptoError::WrongPassphrase,
        other => CryptoError::storage(format!("vault open: {other}")),
    }
}

/// Map a keystore initialization / unlock error to a user-facing string:
/// a passphrase that does not open the vault is "Wrong passphrase"
/// (`CryptoError::WrongPassphrase`, typed at the vault's keying phase);
/// anything else passes its own text through.
pub fn map_stronghold_error(e: &rekindle_crypto::CryptoError) -> String {
    match e {
        rekindle_crypto::CryptoError::WrongPassphrase => {
            "Wrong passphrase — unable to unlock keystore".to_string()
        }
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn keystore(dir: &TempDir, pass: &str) -> Result<StrongholdKeystore, CryptoError> {
        test_keystore(dir.path(), pass)
    }

    #[test]
    fn roundtrip_store_and_load() {
        let dir = TempDir::new().unwrap();
        let ks = keystore(&dir, "testpass").unwrap();

        let secret = [42u8; 32];
        ks.vault_put(&VaultKey::IdentityEd25519, &secret).unwrap();

        let loaded = ks
            .vault_get(&VaultKey::IdentityEd25519)
            .unwrap()
            .expect("key should exist");
        assert_eq!(loaded, secret);
    }

    #[test]
    fn persistence_across_reopen() {
        let dir = TempDir::new().unwrap();
        let secret = [7u8; 32];

        // Write and save
        {
            let ks = keystore(&dir, "pass123").unwrap();
            ks.vault_put(&VaultKey::IdentityEd25519, &secret).unwrap();
        }

        // Reopen and read
        {
            let ks = keystore(&dir, "pass123").unwrap();
            let loaded = ks
                .vault_get(&VaultKey::IdentityEd25519)
                .unwrap()
                .expect("key should persist");
            assert_eq!(loaded, secret);
        }
    }

    #[test]
    fn wrong_passphrase_rejects_snapshot() {
        let dir = TempDir::new().unwrap();
        let secret = [99u8; 32];

        // Create keystore with correct passphrase and save
        {
            let ks = keystore(&dir, "correct-pass").unwrap();
            ks.vault_put(&VaultKey::IdentityEd25519, &secret).unwrap();
        }

        // Attempt to open with wrong passphrase — should fail
        let result = keystore(&dir, "wrong-pass");
        assert!(
            result.is_err(),
            "wrong passphrase should fail to load snapshot"
        );
    }

    #[test]
    fn key_exists_and_delete() {
        let dir = TempDir::new().unwrap();
        let ks = keystore(&dir, "testpass").unwrap();

        assert!(ks.vault_get(&VaultKey::IdentityEd25519).unwrap().is_none());
        ks.vault_put(&VaultKey::IdentityEd25519, &[1u8; 32])
            .unwrap();
        assert!(ks.vault_get(&VaultKey::IdentityEd25519).unwrap().is_some());
        ks.delete(&VaultKey::IdentityEd25519).unwrap();
        assert!(ks.vault_get(&VaultKey::IdentityEd25519).unwrap().is_none());
    }
}
