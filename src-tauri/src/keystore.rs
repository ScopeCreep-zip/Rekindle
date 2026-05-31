use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;

use rekindle_crypto::{CryptoError, Keychain};
use rekindle_vault::VaultStore;

/// VaultStore-backed keystore (Phase 2 of the decomposed-harvest plan
/// replaced the prior `iota_stronghold` backend). The type name is kept
/// as `StrongholdKeystore` so the 30+ persist/load/delete helpers below
/// and the 13 consumer files across src-tauri don't need a rename pass.
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
    pub fn delete_snapshot(
        vault_dir: &Path,
        public_key_hex: &str,
    ) -> Result<(), std::io::Error> {
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
        let vault = VaultStore::open(path, passphrase)
            .map_err(|e| CryptoError::StorageError(format!("vault open: {e}")))?;
        Ok(Self { vault })
    }

    /// No-op for the vault-backed keystore.
    ///
    /// Stronghold required an explicit `commit_with_keyprovider` after
    /// every mutation; SQLCipher writes are immediate (autocommit). The
    /// function is retained because the 30+ persist/delete helpers below
    /// call `keystore.save()` after every Keychain operation and we don't
    /// want to touch those call sites in Phase 2.
    #[allow(
        clippy::unused_self,
        reason = "API parity with the Stronghold-era keystore — 30+ persist_X helpers in this file call keystore.save() after each Keychain op"
    )]
    #[allow(
        clippy::unnecessary_wraps,
        reason = "API parity with the Stronghold-era keystore — call sites use .map_err(...) for error propagation"
    )]
    pub fn save(&self) -> Result<(), CryptoError> {
        Ok(())
    }

    /// Number of entries in the vault — exposed for the dev-only
    /// `vault_diagnostics` Tauri command.
    pub fn entry_count(&self) -> Result<usize, CryptoError> {
        self.vault
            .entry_count()
            .map_err(|e| CryptoError::StorageError(format!("vault count: {e}")))
    }

    /// On-disk path of this vault — exposed for diagnostics. Used to read
    /// the file's mtime for the `last_write_ms` field of vault_diagnostics.
    pub fn vault_path(&self) -> &std::path::Path {
        self.vault.path()
    }
}

impl Keychain for StrongholdKeystore {
    fn store_key(&self, vault: &str, key: &str, data: &[u8]) -> Result<(), CryptoError> {
        self.vault
            .put(vault, key, data)
            .map_err(|e| CryptoError::StorageError(format!("vault put: {e}")))
    }

    fn load_key(&self, vault: &str, key: &str) -> Result<Option<Vec<u8>>, CryptoError> {
        self.vault
            .get(vault, key)
            .map(|opt| opt.map(|z| z.to_vec()))
            .map_err(|e| CryptoError::StorageError(format!("vault get: {e}")))
    }

    fn delete_key(&self, vault: &str, key: &str) -> Result<(), CryptoError> {
        self.vault
            .delete(vault, key)
            .map_err(|e| CryptoError::StorageError(format!("vault delete: {e}")))
    }

    fn key_exists(&self, vault: &str, key: &str) -> Result<bool, CryptoError> {
        self.vault
            .key_exists(vault, key)
            .map_err(|e| CryptoError::StorageError(format!("vault contains: {e}")))
    }
}

/// Map a keystore initialization / unlock error to a user-friendly string.
///
/// Detects the SQLCipher "wrong key" failure pattern (surfaced as
/// `vault open: ...` containing the SQLCipher key validation message)
/// and returns the "Wrong passphrase" prompt; otherwise passes the
/// original error text through.
///
/// Function name is unchanged (`map_stronghold_error`) so the 13 consumer
/// sites and CLAUDE.md references don't break.
pub fn map_stronghold_error(e: &rekindle_crypto::CryptoError) -> String {
    let msg = e.to_string();
    if msg.contains("wrong passphrase")
        || msg.contains("corrupt vault")
        || msg.contains("not a database")
        || msg.contains("SQLCipher")
        || msg.contains("vault open:")
    {
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

// `derive_key` removed in Phase 2 — VaultStore handles Argon2id passphrase
// derivation internally (see `rekindle-vault::store::derive_two_keys`).

// ─── B7/D4 (P0.5): Vault delegate helpers for Signal stores ────────────────
//
// Architecture §11 — Signal sessions/prekeys/identity must persist across
// restart so a corrupted-on-disk session is recoverable, not the default
// state. The previous Memory*Store implementations lost everything on app
// exit, forcing every friend to re-handshake on every launch. For a
// vulnerable user this is a social-engineering opportunity: an attacker
// who can prompt a re-handshake can substitute their own keys.
//
// All helpers below mirror the persist_mek / load_mek pattern (line 185+):
// - take a `&StrongholdKeystore` handle (already locked by caller)
// - use VAULT_SIGNAL from rekindle-crypto::keychain
// - encode key names with stable string prefixes so old/new entries don't
//   collide
//
// Indices: Stronghold has no list-keys API, so for the multi-entry stores
// (sessions, prekeys, trusted identities) we maintain a separate "index"
// record holding the current keys. Index updates are best-effort (log on
// fail); the per-entry persist is the authoritative write.

const SIGNAL_SESSION_PREFIX: &str = "session:";
const SIGNAL_PREKEY_PREFIX: &str = "prekey:";
const SIGNAL_SIGNED_PREKEY_PREFIX: &str = "signed_prekey:";
const SIGNAL_TRUSTED_PREFIX: &str = "trusted:";
const SIGNAL_REGISTRATION_KEY: &str = "registration_id";
const SIGNAL_SESSION_INDEX: &str = "session_index";
const SIGNAL_PREKEY_INDEX: &str = "prekey_index";
const SIGNAL_PQ_LR_PREFIX: &str = "pq_lr:";
const SIGNAL_PQ_OT_PREFIX: &str = "pq_ot:";

/// Persist Signal identity key pair + registration ID. Loaded once at login;
/// the in-memory store mirrors it for fast access.
pub fn persist_signal_identity(
    keystore: &StrongholdKeystore,
    identity_private: &[u8],
    identity_public: &[u8],
    registration_id: u32,
) -> Result<(), String> {
    use rekindle_crypto::keychain::{KEY_SIGNAL_IDENTITY, VAULT_SIGNAL};
    use rekindle_crypto::Keychain as _;

    let mut blob = Vec::with_capacity(identity_private.len() + identity_public.len() + 8);
    blob.extend_from_slice(&u32::try_from(identity_private.len()).unwrap_or(0).to_le_bytes());
    blob.extend_from_slice(identity_private);
    blob.extend_from_slice(&u32::try_from(identity_public.len()).unwrap_or(0).to_le_bytes());
    blob.extend_from_slice(identity_public);
    keystore
        .store_key(VAULT_SIGNAL, KEY_SIGNAL_IDENTITY, &blob)
        .map_err(|e| format!("persist signal identity: {e}"))?;
    keystore
        .store_key(
            VAULT_SIGNAL,
            SIGNAL_REGISTRATION_KEY,
            &registration_id.to_le_bytes(),
        )
        .map_err(|e| format!("persist signal registration_id: {e}"))?;
    keystore
        .save()
        .map_err(|e| format!("save snapshot after signal identity: {e}"))
}

/// Load the persisted Signal identity. Returns None if never persisted.
pub fn load_signal_identity(
    keystore: &StrongholdKeystore,
) -> Option<(Vec<u8>, Vec<u8>, u32)> {
    use rekindle_crypto::keychain::{KEY_SIGNAL_IDENTITY, VAULT_SIGNAL};
    use rekindle_crypto::Keychain as _;

    let blob = keystore.load_key(VAULT_SIGNAL, KEY_SIGNAL_IDENTITY).ok()??;
    if blob.len() < 8 {
        return None;
    }
    let priv_len = u32::from_le_bytes(blob[0..4].try_into().ok()?) as usize;
    if blob.len() < 4 + priv_len + 4 {
        return None;
    }
    let private = blob[4..4 + priv_len].to_vec();
    let pub_len_offset = 4 + priv_len;
    let pub_len =
        u32::from_le_bytes(blob[pub_len_offset..pub_len_offset + 4].try_into().ok()?) as usize;
    if blob.len() < pub_len_offset + 4 + pub_len {
        return None;
    }
    let public = blob[pub_len_offset + 4..pub_len_offset + 4 + pub_len].to_vec();
    let reg_bytes = keystore.load_key(VAULT_SIGNAL, SIGNAL_REGISTRATION_KEY).ok()??;
    let registration_id = u32::from_le_bytes(reg_bytes.as_slice().try_into().ok()?);
    Some((private, public, registration_id))
}

/// Persist a per-peer trusted-identity entry (TOFU).
pub fn persist_trusted_identity(
    keystore: &StrongholdKeystore,
    peer_address: &str,
    identity_key: &[u8],
) -> Result<(), String> {
    use rekindle_crypto::keychain::VAULT_SIGNAL;
    use rekindle_crypto::Keychain as _;

    let key_name = format!("{SIGNAL_TRUSTED_PREFIX}{peer_address}");
    keystore
        .store_key(VAULT_SIGNAL, &key_name, identity_key)
        .map_err(|e| format!("persist trusted identity: {e}"))?;
    keystore
        .save()
        .map_err(|e| format!("save snapshot after trusted identity: {e}"))
}

/// Load the trusted identity for a peer (None if no prior interaction).
pub fn load_trusted_identity(
    keystore: &StrongholdKeystore,
    peer_address: &str,
) -> Option<Vec<u8>> {
    use rekindle_crypto::keychain::VAULT_SIGNAL;
    use rekindle_crypto::Keychain as _;

    let key_name = format!("{SIGNAL_TRUSTED_PREFIX}{peer_address}");
    keystore.load_key(VAULT_SIGNAL, &key_name).ok()?
}

/// Persist a Signal session for a peer, updating the session index so the
/// store can list known peers after restart.
pub fn persist_signal_session(
    keystore: &StrongholdKeystore,
    peer_address: &str,
    session_data: &[u8],
) -> Result<(), String> {
    use rekindle_crypto::keychain::VAULT_SIGNAL;
    use rekindle_crypto::Keychain as _;

    let key_name = format!("{SIGNAL_SESSION_PREFIX}{peer_address}");
    keystore
        .store_key(VAULT_SIGNAL, &key_name, session_data)
        .map_err(|e| format!("persist signal session: {e}"))?;
    add_to_string_index(keystore, SIGNAL_SESSION_INDEX, peer_address)?;
    keystore
        .save()
        .map_err(|e| format!("save snapshot after signal session: {e}"))
}

/// Load a Signal session for a peer (None if no prior session).
pub fn load_signal_session(
    keystore: &StrongholdKeystore,
    peer_address: &str,
) -> Option<Vec<u8>> {
    use rekindle_crypto::keychain::VAULT_SIGNAL;
    use rekindle_crypto::Keychain as _;

    let key_name = format!("{SIGNAL_SESSION_PREFIX}{peer_address}");
    keystore.load_key(VAULT_SIGNAL, &key_name).ok()?
}

/// Delete a Signal session and remove it from the index.
pub fn delete_signal_session(keystore: &StrongholdKeystore, peer_address: &str) {
    use rekindle_crypto::keychain::VAULT_SIGNAL;
    use rekindle_crypto::Keychain as _;

    let key_name = format!("{SIGNAL_SESSION_PREFIX}{peer_address}");
    if let Err(e) = keystore.delete_key(VAULT_SIGNAL, &key_name) {
        tracing::warn!(peer = %peer_address, error = %e, "delete signal session failed");
    }
    let _ = remove_from_string_index(keystore, SIGNAL_SESSION_INDEX, peer_address);
    if let Err(e) = keystore.save() {
        tracing::warn!(error = %e, "save snapshot after signal session delete failed");
    }
}

/// List all peers with persisted Signal sessions (used at login to populate
/// the in-memory cache).
pub fn list_signal_sessions(keystore: &StrongholdKeystore) -> Vec<String> {
    load_string_index(keystore, SIGNAL_SESSION_INDEX)
}

/// Persist a one-time prekey, updating the prekey index.
pub fn persist_signal_prekey(
    keystore: &StrongholdKeystore,
    prekey_id: u32,
    key_data: &[u8],
) -> Result<(), String> {
    use rekindle_crypto::keychain::VAULT_SIGNAL;
    use rekindle_crypto::Keychain as _;

    let key_name = format!("{SIGNAL_PREKEY_PREFIX}{prekey_id}");
    keystore
        .store_key(VAULT_SIGNAL, &key_name, key_data)
        .map_err(|e| format!("persist signal prekey: {e}"))?;
    add_to_string_index(keystore, SIGNAL_PREKEY_INDEX, &prekey_id.to_string())?;
    keystore
        .save()
        .map_err(|e| format!("save snapshot after signal prekey: {e}"))
}

/// Load a one-time prekey by id (None if missing or already consumed).
pub fn load_signal_prekey(keystore: &StrongholdKeystore, prekey_id: u32) -> Option<Vec<u8>> {
    use rekindle_crypto::keychain::VAULT_SIGNAL;
    use rekindle_crypto::Keychain as _;

    let key_name = format!("{SIGNAL_PREKEY_PREFIX}{prekey_id}");
    keystore.load_key(VAULT_SIGNAL, &key_name).ok()?
}

/// Delete a consumed one-time prekey and remove from index.
pub fn delete_signal_prekey(keystore: &StrongholdKeystore, prekey_id: u32) {
    use rekindle_crypto::keychain::VAULT_SIGNAL;
    use rekindle_crypto::Keychain as _;

    let key_name = format!("{SIGNAL_PREKEY_PREFIX}{prekey_id}");
    if let Err(e) = keystore.delete_key(VAULT_SIGNAL, &key_name) {
        tracing::warn!(prekey_id, error = %e, "delete signal prekey failed");
    }
    let _ = remove_from_string_index(keystore, SIGNAL_PREKEY_INDEX, &prekey_id.to_string());
    if let Err(e) = keystore.save() {
        tracing::warn!(error = %e, "save snapshot after signal prekey delete failed");
    }
}

/// List all currently-persisted one-time prekey IDs.
pub fn list_signal_prekey_ids(keystore: &StrongholdKeystore) -> Vec<u32> {
    load_string_index(keystore, SIGNAL_PREKEY_INDEX)
        .into_iter()
        .filter_map(|s| s.parse::<u32>().ok())
        .collect()
}

fn pq_key_name(prekey_id: u32, last_resort: bool) -> String {
    let prefix = if last_resort { SIGNAL_PQ_LR_PREFIX } else { SIGNAL_PQ_OT_PREFIX };
    format!("{prefix}{prekey_id}")
}

/// Persist an ML-KEM-768 secret (PQXDH last-resort or one-time).
pub fn persist_signal_pq_secret(
    keystore: &StrongholdKeystore,
    prekey_id: u32,
    last_resort: bool,
    key_data: &[u8],
) -> Result<(), String> {
    use rekindle_crypto::keychain::VAULT_SIGNAL;
    use rekindle_crypto::Keychain as _;

    let key_name = pq_key_name(prekey_id, last_resort);
    keystore
        .store_key(VAULT_SIGNAL, &key_name, key_data)
        .map_err(|e| format!("persist pq secret: {e}"))?;
    keystore
        .save()
        .map_err(|e| format!("save snapshot after pq secret: {e}"))
}

/// Load an ML-KEM-768 secret by id and kind.
pub fn load_signal_pq_secret(
    keystore: &StrongholdKeystore,
    prekey_id: u32,
    last_resort: bool,
) -> Option<Vec<u8>> {
    use rekindle_crypto::keychain::VAULT_SIGNAL;
    use rekindle_crypto::Keychain as _;

    let key_name = pq_key_name(prekey_id, last_resort);
    keystore.load_key(VAULT_SIGNAL, &key_name).ok()?
}

/// Delete a consumed ML-KEM-768 secret.
pub fn delete_signal_pq_secret(
    keystore: &StrongholdKeystore,
    prekey_id: u32,
    last_resort: bool,
) {
    use rekindle_crypto::keychain::VAULT_SIGNAL;
    use rekindle_crypto::Keychain as _;

    let key_name = pq_key_name(prekey_id, last_resort);
    if let Err(e) = keystore.delete_key(VAULT_SIGNAL, &key_name) {
        tracing::warn!(prekey_id, last_resort, error = %e, "delete pq secret failed");
    }
    if let Err(e) = keystore.save() {
        tracing::warn!(error = %e, "save snapshot after pq secret delete failed");
    }
}

// Phase 4 — audit MAC key persistence. Stored under a dedicated `"audit"`
// namespace so it can't collide with Signal entries. Generated on first
// `load_or_create_audit_mac_key` call; reused on every subsequent unlock.
const AUDIT_NAMESPACE: &str = "audit";
const AUDIT_MAC_KEY_NAME: &str = "mac_key";

/// Load the audit MAC key, generating + persisting a fresh one on first call.
/// Returns 32 bytes suitable for `AuditChain::open`.
///
/// # Errors
/// Returns `String` on vault I/O failure or RNG failure.
pub fn load_or_create_audit_mac_key(
    keystore: &StrongholdKeystore,
) -> Result<[u8; 32], String> {
    use rand::RngCore;
    use rekindle_crypto::Keychain as _;

    // Distinguish three states explicitly:
    //   Ok(Some(32B)) → reuse existing key (idempotent).
    //   Ok(Some(wrong-length)) → corrupt; regenerate (existing chain becomes
    //                            unverifiable — that's the tamper signal).
    //   Ok(None) → no key yet; generate one.
    //   Err(_) → transient I/O failure; refuse to overwrite a potentially
    //            recoverable key, propagate the error so the caller can
    //            either retry or leave the chain disabled.
    match keystore.load_key(AUDIT_NAMESPACE, AUDIT_MAC_KEY_NAME) {
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
    keystore
        .store_key(AUDIT_NAMESPACE, AUDIT_MAC_KEY_NAME, &key_bytes)
        .map_err(|e| format!("persist audit mac key: {e}"))?;
    keystore
        .save()
        .map_err(|e| format!("save snapshot after audit mac key: {e}"))?;
    Ok(key_bytes)
}

const AUDIT_TAIL_KEY_NAME: &str = "tail";

/// Persist the current chain tail (cursor + mac) to the vault. Used as an
/// out-of-band anchor for detecting SQLite-side tail truncation: an attacker
/// can drop trailing `audit_entries` rows, but they cannot forge a matching
/// vault update without the vault passphrase.
///
/// Wire format: 8 bytes LE cursor || 32 bytes mac = 40 bytes total.
///
/// # Errors
/// Returns `String` on vault I/O failure.
pub fn persist_audit_tail(
    keystore: &StrongholdKeystore,
    cursor: u64,
    mac: &[u8; 32],
) -> Result<(), String> {
    use rekindle_crypto::Keychain as _;

    let mut payload = Vec::with_capacity(40);
    payload.extend_from_slice(&cursor.to_le_bytes());
    payload.extend_from_slice(mac);
    keystore
        .store_key(AUDIT_NAMESPACE, AUDIT_TAIL_KEY_NAME, &payload)
        .map_err(|e| format!("persist audit tail: {e}"))?;
    keystore
        .save()
        .map_err(|e| format!("save snapshot after audit tail: {e}"))?;
    Ok(())
}

/// Load the persisted tail anchor written by `persist_audit_tail`, or
/// `None` if no anchor has been written yet (fresh identity).
#[must_use]
pub fn load_audit_tail(keystore: &StrongholdKeystore) -> Option<(u64, [u8; 32])> {
    use rekindle_crypto::Keychain as _;

    let bytes = keystore.load_key(AUDIT_NAMESPACE, AUDIT_TAIL_KEY_NAME).ok()??;
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

/// Persist a signed prekey by id.
pub fn persist_signal_signed_prekey(
    keystore: &StrongholdKeystore,
    signed_prekey_id: u32,
    key_data: &[u8],
) -> Result<(), String> {
    use rekindle_crypto::keychain::VAULT_SIGNAL;
    use rekindle_crypto::Keychain as _;

    let key_name = format!("{SIGNAL_SIGNED_PREKEY_PREFIX}{signed_prekey_id}");
    keystore
        .store_key(VAULT_SIGNAL, &key_name, key_data)
        .map_err(|e| format!("persist signed prekey: {e}"))?;
    keystore
        .save()
        .map_err(|e| format!("save snapshot after signed prekey: {e}"))
}

/// Load a signed prekey by id (None if missing).
pub fn load_signal_signed_prekey(
    keystore: &StrongholdKeystore,
    signed_prekey_id: u32,
) -> Option<Vec<u8>> {
    use rekindle_crypto::keychain::VAULT_SIGNAL;
    use rekindle_crypto::Keychain as _;

    let key_name = format!("{SIGNAL_SIGNED_PREKEY_PREFIX}{signed_prekey_id}");
    keystore.load_key(VAULT_SIGNAL, &key_name).ok()?
}

// ─── String-index helpers (Stronghold has no list-keys API) ─────────────────

fn load_string_index(keystore: &StrongholdKeystore, index_name: &str) -> Vec<String> {
    use rekindle_crypto::keychain::VAULT_SIGNAL;
    use rekindle_crypto::Keychain as _;

    let Ok(Some(blob)) = keystore.load_key(VAULT_SIGNAL, index_name) else {
        return Vec::new();
    };
    serde_json::from_slice::<Vec<String>>(&blob).unwrap_or_default()
}

fn save_string_index(
    keystore: &StrongholdKeystore,
    index_name: &str,
    entries: &[String],
) -> Result<(), String> {
    use rekindle_crypto::keychain::VAULT_SIGNAL;
    use rekindle_crypto::Keychain as _;

    let blob = serde_json::to_vec(entries).map_err(|e| format!("serialize index: {e}"))?;
    keystore
        .store_key(VAULT_SIGNAL, index_name, &blob)
        .map_err(|e| format!("store index: {e}"))
}

fn add_to_string_index(
    keystore: &StrongholdKeystore,
    index_name: &str,
    entry: &str,
) -> Result<(), String> {
    let mut entries = load_string_index(keystore, index_name);
    if !entries.iter().any(|e| e == entry) {
        entries.push(entry.to_string());
        save_string_index(keystore, index_name, &entries)?;
    }
    Ok(())
}

fn remove_from_string_index(
    keystore: &StrongholdKeystore,
    index_name: &str,
    entry: &str,
) -> Result<(), String> {
    let mut entries = load_string_index(keystore, index_name);
    let original_len = entries.len();
    entries.retain(|e| e != entry);
    if entries.len() != original_len {
        save_string_index(keystore, index_name, &entries)?;
    }
    Ok(())
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

    // ── Phase 4 — audit MAC key + tail anchor tests ──────────────────────

    #[test]
    fn audit_mac_key_is_idempotent_across_calls() {
        // Calling load_or_create_audit_mac_key twice must return the SAME
        // bytes — chain.verify() needs key stability across sessions.
        let dir = TempDir::new().unwrap();
        let ks = StrongholdKeystore::initialize(dir.path(), "pp").unwrap();

        let k1 = load_or_create_audit_mac_key(&ks).unwrap();
        let k2 = load_or_create_audit_mac_key(&ks).unwrap();
        assert_eq!(k1, k2, "mac key must be stable across calls");
    }

    #[test]
    fn audit_mac_key_persists_across_keystore_reopen() {
        // Reopening the vault (simulating restart) must yield the same key.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("rekindle.vault");
        let k_first = {
            let ks = StrongholdKeystore::initialize(dir.path(), "pp").unwrap();
            load_or_create_audit_mac_key(&ks).unwrap()
        };
        let k_second = {
            let ks = StrongholdKeystore::initialize(dir.path(), "pp").unwrap();
            load_or_create_audit_mac_key(&ks).unwrap()
        };
        assert_eq!(k_first, k_second, "mac key must survive vault reopen");
        let _ = path; // suppress unused warning
    }

    #[test]
    fn audit_mac_key_regenerates_on_corrupt_length() {
        // If a wrong-length entry exists at ("audit", "mac_key") — a corruption
        // signal — load_or_create_audit_mac_key must regenerate. Existing
        // chain entries become unverifiable; that mismatch IS the tamper signal.
        use rekindle_crypto::Keychain as _;
        let dir = TempDir::new().unwrap();
        let ks = StrongholdKeystore::initialize(dir.path(), "pp").unwrap();

        // Inject a wrong-length key.
        ks.store_key("audit", "mac_key", b"too-short").unwrap();
        ks.save().unwrap();
        let regenerated = load_or_create_audit_mac_key(&ks).unwrap();
        assert_eq!(regenerated.len(), 32);
        // Subsequent calls now stable on the new key.
        assert_eq!(load_or_create_audit_mac_key(&ks).unwrap(), regenerated);
    }

    #[test]
    fn audit_tail_anchor_roundtrip() {
        let dir = TempDir::new().unwrap();
        let ks = StrongholdKeystore::initialize(dir.path(), "pp").unwrap();
        let mac = [9u8; 32];
        persist_audit_tail(&ks, 42, &mac).unwrap();
        let loaded = load_audit_tail(&ks).expect("anchor should exist");
        assert_eq!(loaded, (42, mac));
    }

    #[test]
    fn audit_tail_anchor_corrupt_length_returns_none() {
        // If something corrupts the tail entry length, load_audit_tail
        // must NOT panic — it must return None so restore_chain treats
        // the chain as fresh (no false-positive tamper).
        use rekindle_crypto::Keychain as _;
        let dir = TempDir::new().unwrap();
        let ks = StrongholdKeystore::initialize(dir.path(), "pp").unwrap();
        ks.store_key("audit", "tail", b"only-twelve-").unwrap();
        ks.save().unwrap();
        assert!(load_audit_tail(&ks).is_none());
    }

    #[test]
    fn audit_tail_anchor_missing_returns_none() {
        // Fresh vault has no tail anchor → None (not Err).
        let dir = TempDir::new().unwrap();
        let ks = StrongholdKeystore::initialize(dir.path(), "pp").unwrap();
        assert!(load_audit_tail(&ks).is_none());
    }
}
