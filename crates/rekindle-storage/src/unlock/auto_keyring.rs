//! Zero-password auto-unlock via device-bound KEK in the OS keyring.
//!
//! Security equivalent to the OS login session. If the keyring is
//! unlocked (desktop login), the vault opens without user input.
//! Opt-in only — user explicitly enrolls this method.

use zeroize::Zeroizing;

use crate::error::{StorageError, StorageResult};
use crate::unlock::{MasterKey, VaultUnlock};
use crate::vault::entry_crypto;

const SERVICE: &str = "rekindle";

pub struct AutoKeyringUnlock {
    account: String,
}

impl AutoKeyringUnlock {
    pub fn new(identity_short: &str) -> Self {
        Self {
            account: format!("vault-auto-{identity_short}"),
        }
    }

    fn device_kek() -> Zeroizing<[u8; 32]> {
        let mut input = Vec::new();
        if let Ok(mid) = std::fs::read_to_string("/etc/machine-id") {
            input.extend_from_slice(mid.trim().as_bytes());
        }
        #[cfg(unix)]
        {
            input.extend_from_slice(&rustix::process::getuid().as_raw().to_le_bytes());
        }
        if input.is_empty() {
            input.extend_from_slice(b"no-platform-identity");
        }
        let derived = blake3::derive_key("rekindle v1 device-kek", &input);
        zeroize::Zeroize::zeroize(&mut input);
        Zeroizing::new(derived)
    }
}

impl VaultUnlock for AutoKeyringUnlock {
    fn unlock(&self) -> StorageResult<MasterKey> {
        let entry = keyring::Entry::new(SERVICE, &self.account)
            .map_err(|e| StorageError::KeyringFailed(format!("entry: {e}")))?;
        let wrapped_hex = entry
            .get_password()
            .map_err(|e| StorageError::KeyringFailed(format!("get: {e}")))?;
        let wrapped =
            hex::decode(&wrapped_hex).map_err(|e| StorageError::KeyringFailed(format!("hex: {e}")))?;

        let kek = Self::device_kek();
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
        let kek = Self::device_kek();
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
        "auto-keyring"
    }

    fn display_name(&self) -> &'static str {
        "Auto-Unlock (OS Keyring)"
    }
}
