use std::convert::TryFrom;
use std::path::Path;
use std::sync::Arc;

use iota_stronghold::{KeyProvider, SnapshotPath, Stronghold};
use parking_lot::Mutex;
use zeroize::Zeroizing;

use rekindle_crypto::{CryptoError, Keychain};

/// A Stronghold-backed keystore for securely persisting key material.
///
/// Uses `iota_stronghold` directly: secrets are stored in the client `Store`
/// (key-value pairs encrypted at rest in the snapshot file). The snapshot
/// is encrypted with an `Argon2id`-derived key from the user passphrase.
pub struct StrongholdKeystore {
    /// The underlying Stronghold instance.
    stronghold: Stronghold,
    /// Path to the snapshot file on disk.
    snapshot_path: SnapshotPath,
    /// Key provider derived from passphrase (for snapshot encryption).
    keyprovider: KeyProvider,
    /// Client name used for all operations.
    client_name: Vec<u8>,
}

/// Thread-safe handle to the keystore, stored in Tauri managed state.
pub type KeystoreHandle = Arc<Mutex<Option<StrongholdKeystore>>>;

/// Create a new empty keystore handle (unlocked later with passphrase).
pub fn new_handle() -> KeystoreHandle {
    Arc::new(Mutex::new(None))
}

impl StrongholdKeystore {
    /// Initialize a keystore for a specific identity, loading any existing snapshot.
    ///
    /// Each identity gets its own Stronghold file named `{public_key_hex}.stronghold`.
    /// This is necessary because each identity has its own passphrase — you can't
    /// store multiple passphrases' keys in a single encrypted snapshot.
    pub fn initialize_for_identity(
        snapshot_dir: &Path,
        public_key_hex: &str,
        passphrase: &str,
    ) -> Result<Self, CryptoError> {
        let filename = format!("{public_key_hex}.stronghold");
        let snapshot_file = snapshot_dir.join(filename);
        Self::initialize_from_file(&snapshot_file, passphrase)
    }

    /// Delete the Stronghold snapshot file for a specific identity.
    pub fn delete_snapshot(
        snapshot_dir: &Path,
        public_key_hex: &str,
    ) -> Result<(), std::io::Error> {
        let filename = format!("{public_key_hex}.stronghold");
        let snapshot_file = snapshot_dir.join(filename);
        if snapshot_file.exists() {
            std::fs::remove_file(snapshot_file)?;
        }
        Ok(())
    }

    /// Initialize a keystore using the legacy global snapshot file.
    ///
    /// Kept for backward compatibility with unit tests.
    pub fn initialize(snapshot_dir: &Path, passphrase: &str) -> Result<Self, CryptoError> {
        let snapshot_file = snapshot_dir.join("rekindle.stronghold");
        Self::initialize_from_file(&snapshot_file, passphrase)
    }

    /// Common initialization logic for any snapshot file path.
    fn initialize_from_file(snapshot_file: &Path, passphrase: &str) -> Result<Self, CryptoError> {
        let password_hash = derive_key(passphrase);
        let keyprovider = KeyProvider::try_from(Zeroizing::new(password_hash))
            .map_err(|e| CryptoError::StorageError(format!("key provider init: {e:?}")))?;
        let snapshot_path = SnapshotPath::from_path(snapshot_file);

        let stronghold = Stronghold::default();

        // Load existing snapshot if present
        if snapshot_file.exists() {
            stronghold
                .load_snapshot(&keyprovider, &snapshot_path)
                .map_err(|e| CryptoError::StorageError(format!("load snapshot: {e}")))?;
        }

        let client_name = b"rekindle".to_vec();

        // Try to load the client from the snapshot, or create a new one
        let _client = stronghold
            .load_client(&client_name)
            .or_else(|_| stronghold.create_client(&client_name))
            .map_err(|e| CryptoError::StorageError(format!("client init: {e}")))?;

        Ok(Self {
            stronghold,
            snapshot_path,
            keyprovider,
            client_name,
        })
    }

    /// Persist the current state to the snapshot file on disk.
    pub fn save(&self) -> Result<(), CryptoError> {
        self.stronghold
            .write_client(&self.client_name)
            .map_err(|e| CryptoError::StorageError(format!("write client: {e}")))?;
        self.stronghold
            .commit_with_keyprovider(&self.snapshot_path, &self.keyprovider)
            .map_err(|e| CryptoError::StorageError(format!("commit snapshot: {e}")))?;
        Ok(())
    }
}

impl Keychain for StrongholdKeystore {
    fn store_key(&self, vault: &str, key: &str, data: &[u8]) -> Result<(), CryptoError> {
        let client = self
            .stronghold
            .get_client(&self.client_name)
            .map_err(|e| CryptoError::StorageError(format!("get client: {e}")))?;
        let store = client.store();
        let store_key = format!("{vault}/{key}");
        store
            .insert(store_key.into_bytes(), data.to_vec(), None)
            .map_err(|e| CryptoError::StorageError(format!("store insert: {e}")))?;
        Ok(())
    }

    fn load_key(&self, vault: &str, key: &str) -> Result<Option<Vec<u8>>, CryptoError> {
        let client = self
            .stronghold
            .get_client(&self.client_name)
            .map_err(|e| CryptoError::StorageError(format!("get client: {e}")))?;
        let store = client.store();
        let store_key = format!("{vault}/{key}");
        store
            .get(store_key.as_bytes())
            .map_err(|e| CryptoError::StorageError(format!("store get: {e}")))
    }

    fn delete_key(&self, vault: &str, key: &str) -> Result<(), CryptoError> {
        let client = self
            .stronghold
            .get_client(&self.client_name)
            .map_err(|e| CryptoError::StorageError(format!("get client: {e}")))?;
        let store = client.store();
        let store_key = format!("{vault}/{key}");
        store
            .delete(store_key.as_bytes())
            .map_err(|e| CryptoError::StorageError(format!("store delete: {e}")))?;
        Ok(())
    }

    fn key_exists(&self, vault: &str, key: &str) -> Result<bool, CryptoError> {
        let client = self
            .stronghold
            .get_client(&self.client_name)
            .map_err(|e| CryptoError::StorageError(format!("get client: {e}")))?;
        let store = client.store();
        let store_key = format!("{vault}/{key}");
        store
            .contains_key(store_key.as_bytes())
            .map_err(|e| CryptoError::StorageError(format!("store contains: {e}")))
    }
}

/// Map a Stronghold initialization error to a user-friendly string.
///
/// If the error message mentions "snapshot" or "decrypt", the passphrase
/// was wrong; otherwise, surface the original error text.
pub fn map_stronghold_error(e: &rekindle_crypto::CryptoError) -> String {
    let msg = e.to_string();
    if msg.contains("snapshot") || msg.contains("decrypt") {
        "Wrong passphrase — unable to unlock keystore".to_string()
    } else {
        msg
    }
}

/// Persist a community's MEK to the open Stronghold keystore.
///
/// Serializes as `generation (8 bytes LE) + key (32 bytes)`, stores under
/// `VAULT_COMMUNITIES / mek_<community_id>`, and saves the snapshot.
pub fn persist_mek(
    keystore: &StrongholdKeystore,
    community_id: &str,
    mek: &rekindle_crypto::group::media_key::MediaEncryptionKey,
) {
    use rekindle_crypto::keychain::{mek_key_name, VAULT_COMMUNITIES};
    use rekindle_crypto::Keychain as _;

    let payload = mek.to_wire_bytes();

    let key_name = mek_key_name(community_id);
    if let Err(e) = keystore.store_key(VAULT_COMMUNITIES, &key_name, &payload) {
        tracing::warn!(error = %e, community = %community_id, "failed to persist MEK to Stronghold");
    } else if let Err(e) = keystore.save() {
        tracing::warn!(error = %e, community = %community_id, "failed to save Stronghold snapshot after MEK persist");
    } else {
        tracing::debug!(community = %community_id, "MEK persisted to Stronghold");
    }
}

/// Load a community's MEK from the open Stronghold keystore.
///
/// Returns `Some(mek)` if successfully deserialized, `None` otherwise.
pub fn load_mek(
    keystore: &StrongholdKeystore,
    community_id: &str,
) -> Option<rekindle_crypto::group::media_key::MediaEncryptionKey> {
    use rekindle_crypto::keychain::{mek_key_name, VAULT_COMMUNITIES};
    use rekindle_crypto::Keychain as _;

    let key_name = mek_key_name(community_id);
    match keystore.load_key(VAULT_COMMUNITIES, &key_name) {
        Ok(Some(bytes)) => {
            rekindle_crypto::group::media_key::MediaEncryptionKey::from_wire_bytes(&bytes)
        }
        _ => None,
    }
}

/// Delete a community's MEK from the open Stronghold keystore.
pub fn delete_mek(keystore: &StrongholdKeystore, community_id: &str) {
    use rekindle_crypto::keychain::{mek_key_name, VAULT_COMMUNITIES};
    use rekindle_crypto::Keychain as _;

    let key_name = mek_key_name(community_id);
    if let Err(e) = keystore.delete_key(VAULT_COMMUNITIES, &key_name) {
        tracing::warn!(error = %e, community = %community_id, "failed to delete MEK from Stronghold");
    } else if let Err(e) = keystore.save() {
        tracing::warn!(error = %e, community = %community_id, "failed to save Stronghold snapshot after MEK delete");
    } else {
        tracing::debug!(community = %community_id, "MEK deleted from Stronghold");
    }
}

/// Derive a 32-byte encryption key from a passphrase using `Argon2id`.
///
/// Production: `m=65536, t=3, p=4` (standard security).
/// Test builds: `m=256, t=1, p=1` (fast iteration).
fn derive_key(passphrase: &str) -> Vec<u8> {
    use argon2::{Algorithm, Argon2, Params, Version};

    let salt = b"rekindle-stronghold-salt";

    #[cfg(debug_assertions)]
    let params = Params::new(256, 1, 1, Some(32)).expect("invalid argon2 params");
    #[cfg(not(debug_assertions))]
    let params = Params::new(65536, 3, 4, Some(32)).expect("invalid argon2 params");

    let hasher = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = vec![0u8; 32];
    hasher
        .hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .expect("argon2 hash failed");
    key
}

#[cfg(test)]
mod tests {
    use super::*;
    use rekindle_crypto::keychain::{KEY_ED25519_PRIVATE, VAULT_IDENTITY};
    use tempfile::TempDir;

    #[test]
    fn roundtrip_store_and_load() {
        let dir = TempDir::new().unwrap();
        let ks = StrongholdKeystore::initialize(dir.path(), "testpass").unwrap();

        let secret = [42u8; 32];
        ks.store_key(VAULT_IDENTITY, KEY_ED25519_PRIVATE, &secret)
            .unwrap();
        ks.save().unwrap();

        let loaded = ks
            .load_key(VAULT_IDENTITY, KEY_ED25519_PRIVATE)
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
            ks.store_key(VAULT_IDENTITY, KEY_ED25519_PRIVATE, &secret)
                .unwrap();
            ks.save().unwrap();
        }

        // Reopen and read
        {
            let ks = StrongholdKeystore::initialize(dir.path(), "pass123").unwrap();
            let loaded = ks
                .load_key(VAULT_IDENTITY, KEY_ED25519_PRIVATE)
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
            ks.store_key(VAULT_IDENTITY, KEY_ED25519_PRIVATE, &secret)
                .unwrap();
            ks.save().unwrap();
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

        assert!(!ks.key_exists(VAULT_IDENTITY, KEY_ED25519_PRIVATE).unwrap());
        ks.store_key(VAULT_IDENTITY, KEY_ED25519_PRIVATE, &[1u8; 32])
            .unwrap();
        assert!(ks.key_exists(VAULT_IDENTITY, KEY_ED25519_PRIVATE).unwrap());
        ks.delete_key(VAULT_IDENTITY, KEY_ED25519_PRIVATE).unwrap();
        assert!(!ks.key_exists(VAULT_IDENTITY, KEY_ED25519_PRIVATE).unwrap());
    }
}
