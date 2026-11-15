//! `rekindle_mek_rotation::MekPersist` over the unlocked identity's vault:
//! each scope's MEK per generation (`rekindle_vault::typed::mek`), for
//! both hosts (plan D1; the daemon's keyring copy is gone). The same vault
//! is the daemon's `MekHistory`: a generation its in-memory cache does not
//! hold (an unlock restores only each scope's latest) is read from the
//! store of record, as the desktop's `VaultMekHistory` does (A/B R3-9).

use async_trait::async_trait;
use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_mek_rotation::{MekHistory, MekPersist, MekRotationError};
use rekindle_types::channel_keys::KeyScope;
use rekindle_vault::typed::mek::{load_mek_generation, persist_mek};
use rekindle_vault::VaultHandle;

/// MEK persistence into the vault. `MekPersist` takes and returns the raw
/// 32-byte key; the typed helpers store it with its generation.
pub struct VaultMekPersist {
    vault: VaultHandle,
}

impl VaultMekPersist {
    #[must_use]
    pub fn new(vault: VaultHandle) -> Self {
        Self { vault }
    }
}

fn locked() -> MekRotationError {
    MekRotationError::Persist("the vault is locked".into())
}

#[async_trait]
impl MekPersist for VaultMekPersist {
    async fn store_mek_for_generation(
        &self,
        community_id: &str,
        scope: KeyScope,
        generation: u64,
        key_bytes: Vec<u8>,
    ) -> Result<(), MekRotationError> {
        let key_bytes: [u8; 32] = key_bytes
            .try_into()
            .map_err(|_| MekRotationError::Persist("MEK bytes must be 32 long".into()))?;
        let mek = MediaEncryptionKey::from_bytes(key_bytes, generation);
        let guard = self.vault.lock();
        let vault = guard.as_ref().ok_or_else(locked)?;
        persist_mek(vault, community_id, scope, &mek).map_err(MekRotationError::Persist)
    }

    async fn load_mek_for_generation(
        &self,
        community_id: &str,
        scope: KeyScope,
        generation: u64,
    ) -> Result<Option<Vec<u8>>, MekRotationError> {
        let guard = self.vault.lock();
        let vault = guard.as_ref().ok_or_else(locked)?;
        Ok(load_mek_generation(vault, community_id, scope, generation)
            .map(|m| m.as_bytes().to_vec()))
    }
}

impl MekHistory for VaultMekPersist {
    /// The stored key at exactly `generation`; none while locked.
    fn load(
        &self,
        community_id: &str,
        scope: KeyScope,
        generation: u64,
    ) -> Option<MediaEncryptionKey> {
        let guard = self.vault.lock();
        load_mek_generation(guard.as_ref()?, community_id, scope, generation)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use parking_lot::RwLock;
    use rekindle_crypto::group::media_key::MediaEncryptionKey;
    use rekindle_transport::crypto::mek::MekCache;
    use rekindle_types::channel_keys::{KeyEpoch, KeyScope};
    use rekindle_vault::typed::mek::persist_mek;
    use rekindle_vault::VaultStore;

    /// After an unlock the cache holds only each scope's latest generation
    /// (`restore_stored_meks`); an older generation's posts still open,
    /// from the vault.
    #[test]
    fn a_generation_the_cache_lacks_is_read_from_the_vault() {
        let dir = tempfile::tempdir().unwrap();
        let (vault, _) = VaultStore::create(&dir.path().join("pk.vault"), "pass").unwrap();
        for generation in [1, 2] {
            let mek =
                MediaEncryptionKey::from_bytes([u8::try_from(generation).unwrap(); 32], generation);
            persist_mek(&vault, "c", KeyScope::Community, &mek).unwrap();
        }
        let handle = rekindle_vault::new_handle();
        *handle.lock() = Some(vault);
        let cache = Arc::new(RwLock::new(MekCache::new()));
        cache.write().replace_generation(
            "c",
            KeyScope::Community,
            MediaEncryptionKey::from_bytes([2; 32], 2),
        );
        let keys = crate::daemon::mek_rotation::provider_over(cache, handle.clone());

        let key = |generation| keys.key("c", KeyScope::Community, KeyEpoch(generation));
        assert_eq!(key(2).map(|k| *k), Some([2; 32]), "the cached generation");
        assert_eq!(
            key(1).map(|k| *k),
            Some([1; 32]),
            "an older one, from the vault"
        );
        assert!(key(3).is_none(), "never a generation nobody stored");
        *handle.lock() = None;
        assert!(key(1).is_none(), "nothing from a locked vault");
    }
}
