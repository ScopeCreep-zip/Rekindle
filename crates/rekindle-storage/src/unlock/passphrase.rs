//! Passphrase-based vault unlock via Argon2id.
//!
//! Always available. The recovery fallback — cannot be revoked.
//! Salt stored at `~/.local/state/rekindle/vault.salt`.
//! Wrapped master key at `~/.local/state/rekindle/vault.wrapped`.

use std::path::{Path, PathBuf};

use argon2::{Algorithm, Argon2, Params, Version};
use zeroize::Zeroizing;

use crate::error::{StorageError, StorageResult};
use crate::unlock::{MasterKey, VaultUnlock};
use crate::vault::entry_crypto;

const ARGON2_M_COST: u32 = 65536; // 64 MB
const ARGON2_T_COST: u32 = 3;
const ARGON2_P_COST: u32 = 4;
const SALT_LEN: usize = 16;

pub struct PassphraseUnlock {
    salt_path: PathBuf,
    wrapped_path: PathBuf,
    password: Zeroizing<Vec<u8>>,
}

impl PassphraseUnlock {
    pub fn new(state_dir: &Path, password: &[u8]) -> Self {
        Self {
            salt_path: state_dir.join("vault.salt"),
            wrapped_path: state_dir.join("vault.wrapped"),
            password: Zeroizing::new(password.to_vec()),
        }
    }

    fn derive_kek(&self, salt: &[u8; SALT_LEN]) -> StorageResult<Zeroizing<[u8; 32]>> {
        let params = Params::new(ARGON2_M_COST, ARGON2_T_COST, ARGON2_P_COST, Some(32))
            .map_err(|e| StorageError::PassphraseDerivation(format!("params: {e}")))?;
        let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
        let mut kek = Zeroizing::new([0u8; 32]);
        argon2
            .hash_password_into(&self.password, salt, kek.as_mut())
            .map_err(|e| StorageError::PassphraseDerivation(format!("argon2: {e}")))?;
        Ok(kek)
    }
}

impl VaultUnlock for PassphraseUnlock {
    fn unlock(&self) -> StorageResult<MasterKey> {
        let salt_bytes = std::fs::read(&self.salt_path).map_err(|_| StorageError::SaltCorrupt {
            path: self.salt_path.display().to_string(),
        })?;
        let salt: [u8; SALT_LEN] =
            salt_bytes
                .try_into()
                .map_err(|_| StorageError::SaltCorrupt {
                    path: self.salt_path.display().to_string(),
                })?;

        let kek = self.derive_kek(&salt)?;
        let wrapped = std::fs::read(&self.wrapped_path).map_err(|_| StorageError::MasterKeyUnwrapFailed)?;
        let plain =
            entry_crypto::decrypt(&kek, &wrapped).map_err(|_| StorageError::MasterKeyUnwrapFailed)?;

        if plain.len() != 32 {
            return Err(StorageError::MasterKeyUnwrapFailed);
        }
        let mut mk_bytes = [0u8; 32];
        mk_bytes.copy_from_slice(&plain);
        Ok(MasterKey::from_bytes(mk_bytes))
    }

    fn enroll(&self, master_key: &MasterKey) -> StorageResult<()> {
        let mut salt = [0u8; SALT_LEN];
        aws_lc_rs::rand::SystemRandom::new()
            .fill(&mut salt)
            .map_err(|e| StorageError::RngFailed(format!("{e}")))?;

        let kek = self.derive_kek(&salt)?;
        let wrapped = entry_crypto::encrypt(&kek, master_key.as_bytes())?;

        atomic_write(&self.salt_path, &salt)?;
        atomic_write(&self.wrapped_path, &wrapped)?;
        Ok(())
    }

    fn revoke(&self) -> StorageResult<()> {
        Err(StorageError::UnlockMethodUnavailable {
            method: "passphrase cannot be revoked (recovery fallback)".into(),
        })
    }

    fn is_available(&self) -> bool {
        true
    }

    fn id(&self) -> &'static str {
        "passphrase"
    }

    fn display_name(&self) -> &'static str {
        "Passphrase"
    }
}

use aws_lc_rs::rand::SecureRandom;

fn atomic_write(path: &Path, data: &[u8]) -> StorageResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| StorageError::VaultCreationFailed {
            reason: format!("mkdir: {e}"),
        })?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, data).map_err(|e| StorageError::VaultCreationFailed {
        reason: format!("write {}: {e}", tmp.display()),
    })?;
    std::fs::rename(&tmp, path).map_err(|e| StorageError::VaultCreationFailed {
        reason: format!("rename to {}: {e}", path.display()),
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}
