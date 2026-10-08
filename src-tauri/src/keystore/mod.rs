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

use std::path::{Path, PathBuf};
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
    /// Initialize a keystore for a specific identity, opening or creating
    /// the per-identity vault file `{vault_dir}/{public_key_hex}.vault`.
    pub fn initialize_for_identity(
        vault_dir: &Path,
        public_key_hex: &str,
        passphrase: &str,
    ) -> Result<Self, CryptoError> {
        let path = vault_dir.join(format!("{public_key_hex}.vault"));
        Self::initialize_from_file(&path, passphrase)
    }

    /// Delete the on-disk vault for a specific identity. Removes both
    /// `{pk}.vault` and `{pk}.vault.salt` (sidecar). Missing files are
    /// silently tolerated; only I/O errors propagate.
    pub fn delete_snapshot(vault_dir: &Path, public_key_hex: &str) -> Result<(), std::io::Error> {
        let path = vault_dir.join(format!("{public_key_hex}.vault"));
        let salt_path = {
            let mut p = path.as_os_str().to_owned();
            p.push(".salt");
            PathBuf::from(p)
        };
        for p in [&path, &salt_path] {
            if p.exists() {
                std::fs::remove_file(p)?;
            }
        }
        Ok(())
    }

    /// Initialize a keystore at the legacy unit-test path
    /// `{vault_dir}/rekindle.vault`. Kept so existing tests compile.
    pub fn initialize(vault_dir: &Path, passphrase: &str) -> Result<Self, CryptoError> {
        let path = vault_dir.join("rekindle.vault");
        Self::initialize_from_file(&path, passphrase)
    }

    /// Common initialization: open-or-create a vault at `path` with the
    /// given passphrase. Wrong passphrase fails here (SQLCipher key
    /// validation).
    fn initialize_from_file(path: &Path, passphrase: &str) -> Result<Self, CryptoError> {
        let vault = VaultStore::open(path, passphrase).map_err(|e| match e {
            rekindle_vault::VaultError::WrongPassphrase => CryptoError::WrongPassphrase,
            other => CryptoError::storage(format!("vault open: {other}")),
        })?;
        Ok(Self { vault })
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

    #[test]
    fn roundtrip_store_and_load() {
        let dir = TempDir::new().unwrap();
        let ks = StrongholdKeystore::initialize(dir.path(), "testpass").unwrap();

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
            let ks = StrongholdKeystore::initialize(dir.path(), "pass123").unwrap();
            ks.vault_put(&VaultKey::IdentityEd25519, &secret).unwrap();
        }

        // Reopen and read
        {
            let ks = StrongholdKeystore::initialize(dir.path(), "pass123").unwrap();
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
            let ks = StrongholdKeystore::initialize(dir.path(), "correct-pass").unwrap();
            ks.vault_put(&VaultKey::IdentityEd25519, &secret).unwrap();
        }

        // Attempt to open with wrong passphrase — should fail
        let result = StrongholdKeystore::initialize(dir.path(), "wrong-pass");
        assert!(
            result.is_err(),
            "wrong passphrase should fail to load snapshot"
        );
    }

    #[test]
    fn key_exists_and_delete() {
        let dir = TempDir::new().unwrap();
        let ks = StrongholdKeystore::initialize(dir.path(), "testpass").unwrap();

        assert!(ks.vault_get(&VaultKey::IdentityEd25519).unwrap().is_none());
        ks.vault_put(&VaultKey::IdentityEd25519, &[1u8; 32])
            .unwrap();
        assert!(ks.vault_get(&VaultKey::IdentityEd25519).unwrap().is_some());
        ks.delete(&VaultKey::IdentityEd25519).unwrap();
        assert!(ks.vault_get(&VaultKey::IdentityEd25519).unwrap().is_none());
    }
}
