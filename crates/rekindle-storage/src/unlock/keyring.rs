//! OS keyring fast-path unlock.
//!
//! User types password → BLAKE3 derives KEK (nanoseconds, not Argon2id's ~500ms)
//! → unwraps master key from the KEK-wrapped blob stored in the OS keyring.
//!
//! Raw master key NEVER touches the keyring. Only the AES-256-GCM
//! wrapped blob is stored.

use std::path::{Path, PathBuf};

use zeroize::Zeroizing;

use crate::error::{StorageError, StorageResult};
use crate::unlock::{MasterKey, VaultUnlock};
use crate::vault::entry_crypto;

const SERVICE: &str = "rekindle";

pub struct KeyringUnlock {
    account: String,
    salt_path: PathBuf,
    password: Zeroizing<Vec<u8>>,
}

impl KeyringUnlock {
    pub fn new(state_dir: &Path, identity_short: &str, password: &[u8]) -> Self {
        Self {
            account: format!("vault-key-{identity_short}"),
            salt_path: state_dir.join("vault.salt"),
            password: Zeroizing::new(password.to_vec()),
        }
    }

    fn derive_kek(&self, salt: &[u8]) -> Zeroizing<[u8; 32]> {
        let mut input = Vec::with_capacity(self.password.len() + salt.len());
        input.extend_from_slice(&self.password);
        input.extend_from_slice(salt);
        let derived = blake3::derive_key("rekindle v1 kek-fast", &input);
        zeroize::Zeroize::zeroize(&mut input);
        Zeroizing::new(derived)
    }
}

impl VaultUnlock for KeyringUnlock {
    fn unlock(&self) -> StorageResult<MasterKey> {
        let salt = std::fs::read(&self.salt_path).map_err(|_| StorageError::SaltCorrupt {
            path: self.salt_path.display().to_string(),
        })?;

        let entry = keyring::Entry::new(SERVICE, &self.account)
            .map_err(|e| StorageError::KeyringFailed(format!("entry: {e}")))?;
        let wrapped_hex = entry
            .get_password()
            .map_err(|e| StorageError::KeyringFailed(format!("get: {e}")))?;
        let wrapped =
            hex::decode(&wrapped_hex).map_err(|e| StorageError::KeyringFailed(format!("hex: {e}")))?;

        let kek = self.derive_kek(&salt);
        let plain =
            entry_crypto::decrypt(&kek, &wrapped).map_err(|_| StorageError::MasterKeyUnwrapFailed)?;

        if plain.len() != 32 {
            return Err(StorageError::MasterKeyUnwrapFailed);
        }
        let mut mk = [0u8; 32];
        mk.copy_from_slice(&plain);
        Ok(MasterKey::from_bytes(mk))
    }

    fn enroll(&self, master_key: &MasterKey) -> StorageResult<()> {
        let salt = std::fs::read(&self.salt_path).map_err(|_| StorageError::SaltCorrupt {
            path: self.salt_path.display().to_string(),
        })?;

        let kek = self.derive_kek(&salt);
        let wrapped = entry_crypto::encrypt(&kek, master_key.as_bytes())?;
        let wrapped_hex = hex::encode(&wrapped);

        let entry = keyring::Entry::new(SERVICE, &self.account)
            .map_err(|e| StorageError::KeyringFailed(format!("entry: {e}")))?;
        entry
            .set_password(&wrapped_hex)
            .map_err(|e| StorageError::KeyringFailed(format!("set: {e}")))?;
        Ok(())
    }

    fn revoke(&self) -> StorageResult<()> {
        let entry = keyring::Entry::new(SERVICE, &self.account)
            .map_err(|e| StorageError::KeyringFailed(format!("entry: {e}")))?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(StorageError::KeyringFailed(format!("delete: {e}"))),
        }
    }

    fn is_available(&self) -> bool {
        keyring::Entry::new(SERVICE, "availability-probe")
            .and_then(|e| {
                e.set_password("probe")?;
                e.delete_credential()?;
                Ok(())
            })
            .is_ok()
    }

    fn id(&self) -> &'static str {
        "keyring"
    }

    fn display_name(&self) -> &'static str {
        "OS Keyring (fast)"
    }
}
