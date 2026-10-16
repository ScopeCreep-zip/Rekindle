//! `rekindle_mek_rotation::MekPersist` over the unlocked identity's vault:
//! each scope's MEK per generation (`rekindle_vault::typed::mek`), for
//! both hosts (plan D1; the daemon's keyring copy is gone).

use async_trait::async_trait;
use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_mek_rotation::{MekPersist, MekRotationError};
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
