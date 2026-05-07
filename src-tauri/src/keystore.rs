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

/// Strict variant of `persist_mek` for command handlers that must surface
/// failure to the frontend instead of silently logging.
///
/// Architecture §17 (community/MEK durability across restarts): the cached
/// MEK lives in `state.mek_cache` until persisted to Stronghold. If
/// Stronghold is locked or its snapshot save fails, the MEK survives only
/// in memory — on the next app restart the user can't decrypt channel
/// messages from this generation and has to wait for an MEK request/cascade.
/// Vulnerable users need to know this happened, not see a silent "Joined!"
/// toast. Caller (`commands::community::crud::join_community`) propagates
/// the `Err` through the Tauri IPC boundary; the frontend's toast (A6 fix
/// at handlers/community.handlers.ts:120-126) renders the message.
pub fn persist_mek_strict(
    keystore: &StrongholdKeystore,
    community_id: &str,
    mek: &rekindle_crypto::group::media_key::MediaEncryptionKey,
) -> Result<(), String> {
    use rekindle_crypto::keychain::{mek_key_name, VAULT_COMMUNITIES};
    use rekindle_crypto::Keychain as _;

    let payload = mek.to_wire_bytes();
    let key_name = mek_key_name(community_id);
    keystore
        .store_key(VAULT_COMMUNITIES, &key_name, &payload)
        .map_err(|e| {
            format!("Stronghold locked or busy — MEK not persisted (will be lost on restart): {e}")
        })?;
    keystore
        .save()
        .map_err(|e| {
            format!("Stronghold snapshot save failed — MEK in memory only (will be lost on restart): {e}")
        })?;
    tracing::debug!(community = %community_id, "MEK persisted to Stronghold (strict)");
    Ok(())
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

/// Persist a community SMPL slot keypair to Stronghold.
///
/// The slot keypair lets a member write their signed presence to their
/// assigned slot in the member registry SMPL record.
pub fn persist_slot_keypair(keystore: &StrongholdKeystore, community_id: &str, keypair_str: &str) {
    use rekindle_crypto::keychain::VAULT_COMMUNITIES;
    use rekindle_crypto::Keychain as _;

    let key_name = format!("slot_keypair_{community_id}");
    if let Err(e) = keystore.store_key(VAULT_COMMUNITIES, &key_name, keypair_str.as_bytes()) {
        tracing::warn!(error = %e, community = %community_id, "failed to persist slot keypair");
    } else if let Err(e) = keystore.save() {
        tracing::warn!(error = %e, community = %community_id, "failed to save snapshot after slot keypair");
    } else {
        tracing::debug!(community = %community_id, "slot keypair persisted to Stronghold");
    }
}

/// Load a community SMPL slot keypair from Stronghold.
pub fn load_slot_keypair(keystore: &StrongholdKeystore, community_id: &str) -> Option<String> {
    use rekindle_crypto::keychain::VAULT_COMMUNITIES;
    use rekindle_crypto::Keychain as _;

    let key_name = format!("slot_keypair_{community_id}");
    match keystore.load_key(VAULT_COMMUNITIES, &key_name) {
        Ok(Some(bytes)) => String::from_utf8(bytes).ok(),
        Ok(None) => None,
        Err(e) => {
            tracing::trace!(error = %e, community = %community_id, "no slot keypair in Stronghold");
            None
        }
    }
}

/// Delete a community SMPL slot keypair from Stronghold.
pub fn delete_slot_keypair(keystore: &StrongholdKeystore, community_id: &str) {
    use rekindle_crypto::keychain::VAULT_COMMUNITIES;
    use rekindle_crypto::Keychain as _;

    let key_name = format!("slot_keypair_{community_id}");
    if let Err(e) = keystore.delete_key(VAULT_COMMUNITIES, &key_name) {
        tracing::warn!(error = %e, community = %community_id, "failed to delete slot keypair");
    } else if let Err(e) = keystore.save() {
        tracing::warn!(error = %e, community = %community_id, "failed to save snapshot after slot keypair delete");
    }
}

/// Persist the registry owner keypair for a community to the open Stronghold keystore.
pub fn persist_registry_keypair(
    keystore: &StrongholdKeystore,
    community_id: &str,
    keypair_str: &str,
) {
    use rekindle_crypto::keychain::VAULT_COMMUNITIES;
    use rekindle_crypto::Keychain as _;

    let key_name = format!("registry_keypair_{community_id}");
    if let Err(e) = keystore.store_key(VAULT_COMMUNITIES, &key_name, keypair_str.as_bytes()) {
        tracing::warn!(error = %e, community = %community_id, "failed to persist registry keypair");
    } else if let Err(e) = keystore.save() {
        tracing::warn!(error = %e, community = %community_id, "failed to save snapshot after registry keypair persist");
    }
}

/// Load the registry owner keypair for a community from the open Stronghold keystore.
pub fn load_registry_keypair(keystore: &StrongholdKeystore, community_id: &str) -> Option<String> {
    use rekindle_crypto::keychain::VAULT_COMMUNITIES;
    use rekindle_crypto::Keychain as _;

    let key_name = format!("registry_keypair_{community_id}");
    match keystore.load_key(VAULT_COMMUNITIES, &key_name) {
        Ok(Some(bytes)) => String::from_utf8(bytes).ok(),
        Ok(None) => None,
        Err(e) => {
            tracing::trace!(error = %e, community = %community_id, "no registry keypair in keystore");
            None
        }
    }
}

/// Delete the registry owner keypair for a community from the open Stronghold keystore.
pub fn delete_registry_keypair(keystore: &StrongholdKeystore, community_id: &str) {
    use rekindle_crypto::keychain::VAULT_COMMUNITIES;
    use rekindle_crypto::Keychain as _;

    let key_name = format!("registry_keypair_{community_id}");
    if let Err(e) = keystore.delete_key(VAULT_COMMUNITIES, &key_name) {
        tracing::warn!(error = %e, community = %community_id, "failed to delete registry keypair");
    } else if let Err(e) = keystore.save() {
        tracing::warn!(error = %e, community = %community_id, "failed to save snapshot after registry keypair delete");
    }
}

/// Persist the slot seed (hex-encoded 32 bytes) for a community to the open Stronghold keystore.
pub fn persist_slot_seed(keystore: &StrongholdKeystore, community_id: &str, seed_hex: &str) {
    use rekindle_crypto::keychain::VAULT_COMMUNITIES;
    use rekindle_crypto::Keychain as _;

    let key_name = format!("slot_seed_{community_id}");
    if let Err(e) = keystore.store_key(VAULT_COMMUNITIES, &key_name, seed_hex.as_bytes()) {
        tracing::warn!(error = %e, community = %community_id, "failed to persist slot seed");
    } else if let Err(e) = keystore.save() {
        tracing::warn!(error = %e, community = %community_id, "failed to save snapshot after slot seed persist");
    }
}

/// Load the slot seed for a community from the open Stronghold keystore.
pub fn load_slot_seed(keystore: &StrongholdKeystore, community_id: &str) -> Option<String> {
    use rekindle_crypto::keychain::VAULT_COMMUNITIES;
    use rekindle_crypto::Keychain as _;

    let key_name = format!("slot_seed_{community_id}");
    match keystore.load_key(VAULT_COMMUNITIES, &key_name) {
        Ok(Some(bytes)) => String::from_utf8(bytes).ok(),
        Ok(None) => None,
        Err(e) => {
            tracing::trace!(error = %e, community = %community_id, "no slot seed in keystore");
            None
        }
    }
}

/// Delete the slot seed for a community from the open Stronghold keystore.
pub fn delete_slot_seed(keystore: &StrongholdKeystore, community_id: &str) {
    use rekindle_crypto::keychain::VAULT_COMMUNITIES;
    use rekindle_crypto::Keychain as _;

    let key_name = format!("slot_seed_{community_id}");
    if let Err(e) = keystore.delete_key(VAULT_COMMUNITIES, &key_name) {
        tracing::warn!(error = %e, community = %community_id, "failed to delete slot seed");
    } else if let Err(e) = keystore.save() {
        tracing::warn!(error = %e, community = %community_id, "failed to save snapshot after slot seed delete");
    }
}

// persist_channel_log_keypair, load_channel_log_keypair, delete_channel_log_keypair
// removed — SMPL channel records use the shared slot seed. Members derive their
// writer keypair via derive_slot_veilid_keypair(seed, slot_index).

/// Persist a per-channel MEK to the open Stronghold keystore.
///
/// Uses the key format `mek_{community_id}_{channel_id}` under the communities vault.
pub fn persist_channel_mek(
    keystore: &StrongholdKeystore,
    community_id: &str,
    channel_id: &str,
    mek: &rekindle_crypto::group::media_key::MediaEncryptionKey,
) {
    use rekindle_crypto::keychain::VAULT_COMMUNITIES;
    use rekindle_crypto::Keychain as _;

    let payload = mek.to_wire_bytes();
    let key_name = format!("mek_{community_id}_{channel_id}");

    if let Err(e) = keystore.store_key(VAULT_COMMUNITIES, &key_name, &payload) {
        tracing::warn!(
            error = %e, community = %community_id, channel = %channel_id,
            "failed to persist channel MEK to Stronghold"
        );
    } else if let Err(e) = keystore.save() {
        tracing::warn!(
            error = %e, community = %community_id, channel = %channel_id,
            "failed to save Stronghold snapshot after channel MEK persist"
        );
    } else {
        tracing::debug!(
            community = %community_id, channel = %channel_id,
            "channel MEK persisted to Stronghold"
        );
    }
}

/// Load a per-channel MEK from the open Stronghold keystore.
///
/// Returns `Some(mek)` if successfully deserialized, `None` otherwise.
pub fn load_channel_mek(
    keystore: &StrongholdKeystore,
    community_id: &str,
    channel_id: &str,
) -> Option<rekindle_crypto::group::media_key::MediaEncryptionKey> {
    use rekindle_crypto::keychain::VAULT_COMMUNITIES;
    use rekindle_crypto::Keychain as _;

    let key_name = format!("mek_{community_id}_{channel_id}");
    match keystore.load_key(VAULT_COMMUNITIES, &key_name) {
        Ok(Some(bytes)) => {
            rekindle_crypto::group::media_key::MediaEncryptionKey::from_wire_bytes(&bytes)
        }
        _ => None,
    }
}

/// Delete a per-channel MEK from the open Stronghold keystore.
pub fn delete_channel_mek(keystore: &StrongholdKeystore, community_id: &str, channel_id: &str) {
    use rekindle_crypto::keychain::VAULT_COMMUNITIES;
    use rekindle_crypto::Keychain as _;

    let key_name = format!("mek_{community_id}_{channel_id}");
    if let Err(e) = keystore.delete_key(VAULT_COMMUNITIES, &key_name) {
        tracing::warn!(
            error = %e, community = %community_id, channel = %channel_id,
            "failed to delete channel MEK from Stronghold"
        );
    } else if let Err(e) = keystore.save() {
        tracing::warn!(
            error = %e, community = %community_id, channel = %channel_id,
            "failed to save Stronghold snapshot after channel MEK delete"
        );
    } else {
        tracing::debug!(
            community = %community_id, channel = %channel_id,
            "channel MEK deleted from Stronghold"
        );
    }
}

// ── Per-generation MEK persistence ──
//
// When MEK rotates (member departure), the old generation must remain in
// Stronghold so historical messages stay decryptable. Each generation gets
// its own key: `mek_{community}_{channel}_{generation}`. A metadata key
// `mek_generations_{community}_{channel}` tracks known generation numbers
// (Stronghold has no prefix iteration).

/// Persist a channel MEK at a specific generation to Stronghold.
///
/// Also updates the generations-list metadata key so `load_all_channel_mek_generations`
/// can enumerate all stored generations without prefix scanning.
pub fn persist_channel_mek_generation(
    keystore: &StrongholdKeystore,
    community_id: &str,
    channel_id: &str,
    mek: &rekindle_crypto::group::media_key::MediaEncryptionKey,
) {
    use rekindle_crypto::keychain::VAULT_COMMUNITIES;
    use rekindle_crypto::Keychain as _;

    let generation = mek.generation();
    let payload = mek.to_wire_bytes();
    let key_name = format!("mek_{community_id}_{channel_id}_{generation}");

    if let Err(e) = keystore.store_key(VAULT_COMMUNITIES, &key_name, &payload) {
        tracing::warn!(
            error = %e, community = %community_id, channel = %channel_id, generation,
            "failed to persist channel MEK generation to Stronghold"
        );
        return;
    }

    // Update the generations index
    update_generations_index(keystore, community_id, channel_id, generation);

    // Also update the "latest" key for backward compatibility with existing code
    let latest_key = format!("mek_{community_id}_{channel_id}");
    let _ = keystore.store_key(VAULT_COMMUNITIES, &latest_key, &payload);

    if let Err(e) = keystore.save() {
        tracing::warn!(
            error = %e, community = %community_id, channel = %channel_id, generation,
            "failed to save Stronghold snapshot after MEK generation persist"
        );
    } else {
        tracing::debug!(
            community = %community_id, channel = %channel_id, generation,
            "channel MEK generation persisted to Stronghold"
        );
    }
}

/// Persist either a community-level or channel-level MEK.
pub fn store_mek(
    keystore: &StrongholdKeystore,
    community_id: &str,
    channel_id: Option<&str>,
    mek: &rekindle_crypto::group::media_key::MediaEncryptionKey,
) {
    if let Some(channel_id) = channel_id {
        persist_channel_mek_generation(keystore, community_id, channel_id, mek);
    } else {
        persist_mek(keystore, community_id, mek);
    }
}

/// Load a specific MEK generation for a channel from Stronghold.
pub fn load_channel_mek_generation(
    keystore: &StrongholdKeystore,
    community_id: &str,
    channel_id: &str,
    generation: u64,
) -> Option<rekindle_crypto::group::media_key::MediaEncryptionKey> {
    use rekindle_crypto::keychain::VAULT_COMMUNITIES;
    use rekindle_crypto::Keychain as _;

    let key_name = format!("mek_{community_id}_{channel_id}_{generation}");
    match keystore.load_key(VAULT_COMMUNITIES, &key_name) {
        Ok(Some(bytes)) => {
            rekindle_crypto::group::media_key::MediaEncryptionKey::from_wire_bytes(&bytes)
        }
        _ => None,
    }
}

/// Load all persisted MEK generations for a channel from Stronghold.
///
/// Returns MEKs sorted by generation (ascending). Used on startup to
/// populate `channel_mek_cache` so historical messages remain decryptable.
pub fn load_all_channel_mek_generations(
    keystore: &StrongholdKeystore,
    community_id: &str,
    channel_id: &str,
) -> Vec<rekindle_crypto::group::media_key::MediaEncryptionKey> {
    let generations = load_generations_index(keystore, community_id, channel_id);
    let mut meks = Vec::new();
    for gen in generations {
        if let Some(mek) = load_channel_mek_generation(keystore, community_id, channel_id, gen) {
            meks.push(mek);
        }
    }
    meks.sort_by_key(rekindle_crypto::group::media_key::MediaEncryptionKey::generation);
    meks
}

/// Load all known MEKs for a community or channel.
pub fn load_all_meks(
    keystore: &StrongholdKeystore,
    community_id: &str,
    channel_id: Option<&str>,
) -> Vec<rekindle_crypto::group::media_key::MediaEncryptionKey> {
    if let Some(channel_id) = channel_id {
        return load_all_channel_mek_generations(keystore, community_id, channel_id);
    }
    load_mek(keystore, community_id).into_iter().collect()
}

/// Update the generations-index metadata key with a new generation number.
fn update_generations_index(
    keystore: &StrongholdKeystore,
    community_id: &str,
    channel_id: &str,
    generation: u64,
) {
    use rekindle_crypto::keychain::VAULT_COMMUNITIES;
    use rekindle_crypto::Keychain as _;

    let index_key = format!("mek_generations_{community_id}_{channel_id}");
    let mut generations = load_generations_index(keystore, community_id, channel_id);

    if !generations.contains(&generation) {
        generations.push(generation);
        generations.sort_unstable();
    }

    // Serialize as JSON array of u64
    let payload = serde_json::to_vec(&generations).unwrap_or_default();
    if let Err(e) = keystore.store_key(VAULT_COMMUNITIES, &index_key, &payload) {
        tracing::warn!(
            error = %e, community = %community_id, channel = %channel_id,
            "failed to update MEK generations index"
        );
    }
}

/// Load the generations-index for a channel from Stronghold.
fn load_generations_index(
    keystore: &StrongholdKeystore,
    community_id: &str,
    channel_id: &str,
) -> Vec<u64> {
    use rekindle_crypto::keychain::VAULT_COMMUNITIES;
    use rekindle_crypto::Keychain as _;

    let index_key = format!("mek_generations_{community_id}_{channel_id}");
    match keystore.load_key(VAULT_COMMUNITIES, &index_key) {
        Ok(Some(bytes)) => serde_json::from_slice(&bytes).unwrap_or_default(),
        _ => Vec::new(),
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

    #[test]
    fn per_generation_mek_roundtrip() {
        use rekindle_crypto::group::media_key::MediaEncryptionKey;

        let dir = TempDir::new().unwrap();
        let ks = StrongholdKeystore::initialize(dir.path(), "testpass").unwrap();

        let mek_gen1 = MediaEncryptionKey::generate(1);
        let mek_gen2 = MediaEncryptionKey::generate(2);

        // Persist two generations
        persist_channel_mek_generation(&ks, "comm_a", "ch_01", &mek_gen1);
        persist_channel_mek_generation(&ks, "comm_a", "ch_01", &mek_gen2);

        // Load specific generation
        let loaded = load_channel_mek_generation(&ks, "comm_a", "ch_01", 1).unwrap();
        assert_eq!(loaded.generation(), 1);
        assert_eq!(loaded.as_bytes(), mek_gen1.as_bytes());

        let loaded = load_channel_mek_generation(&ks, "comm_a", "ch_01", 2).unwrap();
        assert_eq!(loaded.generation(), 2);
        assert_eq!(loaded.as_bytes(), mek_gen2.as_bytes());

        // Nonexistent generation returns None
        assert!(load_channel_mek_generation(&ks, "comm_a", "ch_01", 99).is_none());
    }

    #[test]
    fn load_all_generations_returns_sorted() {
        use rekindle_crypto::group::media_key::MediaEncryptionKey;

        let dir = TempDir::new().unwrap();
        let ks = StrongholdKeystore::initialize(dir.path(), "testpass").unwrap();

        // Persist out of order
        let mek3 = MediaEncryptionKey::generate(3);
        let mek1 = MediaEncryptionKey::generate(1);
        let mek2 = MediaEncryptionKey::generate(2);

        persist_channel_mek_generation(&ks, "comm_b", "ch_02", &mek3);
        persist_channel_mek_generation(&ks, "comm_b", "ch_02", &mek1);
        persist_channel_mek_generation(&ks, "comm_b", "ch_02", &mek2);

        let all = load_all_channel_mek_generations(&ks, "comm_b", "ch_02");
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].generation(), 1);
        assert_eq!(all[1].generation(), 2);
        assert_eq!(all[2].generation(), 3);
    }

    #[test]
    fn per_generation_survives_reopen() {
        use rekindle_crypto::group::media_key::MediaEncryptionKey;

        let dir = TempDir::new().unwrap();
        let mek = MediaEncryptionKey::generate(42);
        let key_bytes = mek.as_bytes().to_vec();

        // Persist and close
        {
            let ks = StrongholdKeystore::initialize(dir.path(), "pass").unwrap();
            persist_channel_mek_generation(&ks, "comm_c", "ch_03", &mek);
        }

        // Reopen and verify
        {
            let ks = StrongholdKeystore::initialize(dir.path(), "pass").unwrap();
            let loaded = load_channel_mek_generation(&ks, "comm_c", "ch_03", 42).unwrap();
            assert_eq!(loaded.generation(), 42);
            assert_eq!(loaded.as_bytes(), key_bytes.as_slice());
        }
    }

    #[test]
    fn different_channels_isolated() {
        use rekindle_crypto::group::media_key::MediaEncryptionKey;

        let dir = TempDir::new().unwrap();
        let ks = StrongholdKeystore::initialize(dir.path(), "testpass").unwrap();

        let mek_a = MediaEncryptionKey::generate(1);
        let mek_b = MediaEncryptionKey::generate(1);

        persist_channel_mek_generation(&ks, "comm_d", "ch_alpha", &mek_a);
        persist_channel_mek_generation(&ks, "comm_d", "ch_beta", &mek_b);

        // Each channel has its own generation namespace
        let all_alpha = load_all_channel_mek_generations(&ks, "comm_d", "ch_alpha");
        let all_beta = load_all_channel_mek_generations(&ks, "comm_d", "ch_beta");

        assert_eq!(all_alpha.len(), 1);
        assert_eq!(all_beta.len(), 1);
        assert_eq!(all_alpha[0].as_bytes(), mek_a.as_bytes());
        assert_eq!(all_beta[0].as_bytes(), mek_b.as_bytes());
    }

    #[test]
    fn load_all_meks_wraps_community_and_channel_variants() {
        use rekindle_crypto::group::media_key::MediaEncryptionKey;

        let dir = TempDir::new().unwrap();
        let ks = StrongholdKeystore::initialize(dir.path(), "testpass").unwrap();

        let community_mek = MediaEncryptionKey::generate(3);
        let channel_mek_a = MediaEncryptionKey::generate(1);
        let channel_mek_b = MediaEncryptionKey::generate(2);

        persist_mek(&ks, "comm_e", &community_mek);
        persist_channel_mek_generation(&ks, "comm_e", "ch_01", &channel_mek_b);
        persist_channel_mek_generation(&ks, "comm_e", "ch_01", &channel_mek_a);

        let community_all = load_all_meks(&ks, "comm_e", None);
        assert_eq!(community_all.len(), 1);
        assert_eq!(community_all[0].generation(), 3);

        let channel_all = load_all_meks(&ks, "comm_e", Some("ch_01"));
        assert_eq!(channel_all.len(), 2);
        assert_eq!(channel_all[0].generation(), 1);
        assert_eq!(channel_all[1].generation(), 2);
    }
}
