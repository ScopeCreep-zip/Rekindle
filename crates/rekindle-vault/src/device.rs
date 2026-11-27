//! The device vault (plan E3.2 S7d-keys.7, ADR 0022 rule 4): a second store
//! beside the identity vault for the secrets that belong to this device
//! only (`VaultKey::DeviceOpKey`, and the per-device owners moved here).
//! Identity export (`VaultStore::export_files`) never reads this one and
//! strips the device-scoped entries from its copy of the identity vault
//! (`VaultKey::is_device_scoped`, this vault's key among them), so a device
//! key never travels with an export and an imported identity enrols new keys.
//!
//! Its random 32-byte key is held in the identity vault under
//! `VaultKey::DeviceVaultSecret`, so one Argon2 unlock opens both (decided:
//! one passphrase stretch per login, not two); it has no `.hdr`.

use std::path::Path;

use zeroize::Zeroizing;

use crate::{VaultError, VaultKey, VaultStore};

/// Open this device's vault at `path` with the key `identity` holds,
/// right after the identity vault opened.
///
/// An identity vault holding no key and no device vault at `path` is a
/// first unlock on this device, an imported identity among them (the export
/// carries neither the key nor the file); a key with no file is an identity
/// vault copied without its device vault. Either mints a fresh key
/// and an empty device vault: this device is new to the identity, as ADR
/// 0022 rule 4 requires.
///
/// # Errors
/// A device vault at `path` whose key the identity vault does not hold
/// ([`VaultError::WrongPassphrase`]: nothing can open it), or the vault
/// I/O, SQLite or AEAD failures.
pub fn open(identity: &VaultStore, path: &Path) -> Result<VaultStore, VaultError> {
    let held = identity.get(&VaultKey::DeviceVaultSecret)?;
    match held {
        Some(bytes) if path.exists() => {
            let key: Zeroizing<[u8; 32]> = Zeroizing::new(
                bytes
                    .as_slice()
                    .try_into()
                    .map_err(|_| VaultError::Schema("device vault key is not 32 bytes".into()))?,
            );
            VaultStore::open_raw(path, &key, false)
        }
        None if path.exists() => Err(VaultError::WrongPassphrase),
        Some(_) | None => {
            let mut key = Zeroizing::new([0u8; 32]);
            rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, key.as_mut_slice());
            let created = VaultStore::open_raw(path, &key, true)?;
            identity.put(&VaultKey::DeviceVaultSecret, key.as_slice())?;
            Ok(created)
        }
    }
}

#[cfg(test)]
#[path = "device_tests.rs"]
mod tests;
