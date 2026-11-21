//! The device vault (plan E3.2 S7d-keys.7, ADR 0022 rule 4): a second store
//! beside the identity vault for the secrets that belong to this device
//! only (`VaultKey::DeviceOpKey`, and the per-device owners moved here).
//! Identity export copies the identity vault file whole
//! (`VaultStore::export_files`) and never reads this one, so a device key
//! never travels with an export and an imported identity enrols new keys.
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
/// first unlock on this device; a key with no file is an imported identity
/// (the export carried the key, never the file). Either mints a fresh key
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
mod tests {
    use super::*;

    fn op_key(community: &str) -> VaultKey {
        VaultKey::DeviceOpKey {
            community: community.into(),
        }
    }

    #[test]
    fn the_device_vault_opens_after_the_identity_vault_with_one_argon2() {
        let dir = tempfile::tempdir().unwrap();
        let (path, device) = (
            dir.path().join("a.vault"),
            dir.path().join("a.device.vault"),
        );
        let (identity, _) = VaultStore::create(&path, "pass").unwrap();
        let first = open(&identity, &device).unwrap();
        first.put(&op_key("c1"), b"device key").unwrap();
        drop(first);
        drop(identity);
        // One passphrase stretch: the identity vault's. The device vault
        // has no header (no Argon2 parameters) and opens from the key the
        // identity vault holds.
        assert!(!crate::header::header_path(&device).exists());
        let (identity, _) = VaultStore::open(&path, "pass").unwrap();
        let again = open(&identity, &device).unwrap();
        assert_eq!(
            again.get(&op_key("c1")).unwrap().unwrap().as_slice(),
            b"device key"
        );
        // Another identity's vault holds no key to it.
        let (other, _) = VaultStore::create(&dir.path().join("b.vault"), "pass").unwrap();
        assert!(matches!(
            open(&other, &device),
            Err(VaultError::WrongPassphrase)
        ));
    }

    #[test]
    fn an_identity_export_never_carries_the_device_vault() {
        let dir = tempfile::tempdir().unwrap();
        let (path, device) = (
            dir.path().join("a.vault"),
            dir.path().join("a.device.vault"),
        );
        let (identity, _) = VaultStore::create(&path, "pass").unwrap();
        let held = open(&identity, &device).unwrap();
        held.put(&op_key("c1"), b"device key").unwrap();
        let (vault_bytes, header_bytes) = identity.export_files().unwrap();
        // The export's vault holds no device secret.
        let imported = dir.path().join("b/a.vault");
        VaultStore::import_files(&imported, &vault_bytes, &header_bytes).unwrap();
        let (copy, _) = VaultStore::open(&imported, "pass").unwrap();
        assert!(copy.list("device").unwrap().is_empty());
        assert_eq!(copy.get(&op_key("c1")).unwrap(), None);
        // The imported identity has no device vault: it mints its own, under
        // a fresh key, holding nothing of the exporting device's.
        let imported_device = dir.path().join("b/a.device.vault");
        let fresh = open(&copy, &imported_device).unwrap();
        assert_eq!(fresh.get(&op_key("c1")).unwrap(), None);
        assert_ne!(
            copy.get(&VaultKey::DeviceVaultSecret).unwrap(),
            identity.get(&VaultKey::DeviceVaultSecret).unwrap()
        );
        // The exporting device still opens its own.
        let own = open(&identity, &device).unwrap();
        assert_eq!(
            own.get(&op_key("c1")).unwrap().unwrap().as_slice(),
            b"device key"
        );
    }
}
