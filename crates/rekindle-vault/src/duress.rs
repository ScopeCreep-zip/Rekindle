//! The duress passphrase (plan E7.1, quick wipe).
//!
//! Arming writes `Argon2id(duress passphrase, duress_salt, the header's
//! params)[..32]` into the header's duress tag under a fresh duress salt;
//! disarming refills both with random bytes, so an armed header and an
//! unarmed one look alike. Every open ([`crate::VaultStore::open`])
//! computes the duress derivation first and the real one second, always
//! both, so an open takes the same time armed or not; a match is
//! [`crate::VaultError::Duress`], and the Host wipes the root.

use std::path::Path;

use rand::RngCore;
use subtle::ConstantTimeEq;

use crate::error::VaultError;
use crate::header::{self, VaultHeader};

/// The tag `passphrase` gives under `header`'s duress salt.
fn tag(passphrase: &str, header: &VaultHeader) -> Result<[u8; 32], VaultError> {
    let stretched = header.stretch(passphrase, &header.duress_salt)?;
    let mut out = [0u8; 32];
    out.copy_from_slice(&stretched[..32]);
    Ok(out)
}

/// Whether `passphrase` is the duress passphrase `header` holds, compared
/// in constant time. Runs the full Argon2id whether armed or not.
///
/// # Errors
/// Parameters `argon2` refuses (a corrupt header).
pub(crate) fn matches(passphrase: &str, header: &VaultHeader) -> Result<bool, VaultError> {
    let mut candidate = tag(passphrase, header)?;
    let equal = candidate.ct_eq(&header.duress_tag).into();
    zeroize::Zeroize::zeroize(&mut candidate);
    Ok(equal)
}

/// Arm `duress_passphrase` on the vault at `vault_path`: a fresh duress
/// salt and its tag, one atomic header rewrite. The caller has checked
/// that it is not the vault's own passphrase.
///
/// # Errors
/// No header, a corrupt one, or I/O.
pub fn arm(vault_path: &Path, duress_passphrase: &str) -> Result<(), VaultError> {
    let mut header = header::read(vault_path)?;
    rand::rngs::OsRng.fill_bytes(&mut header.duress_salt);
    header.duress_tag = tag(duress_passphrase, &header)?;
    header::rewrite(vault_path, &header)
}

/// Disarm: random bytes in the duress pair again, one atomic rewrite.
///
/// # Errors
/// No header, a corrupt one, or I/O.
pub fn disarm(vault_path: &Path) -> Result<(), VaultError> {
    let mut header = header::read(vault_path)?;
    header.randomize_duress();
    header::rewrite(vault_path, &header)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::VaultStore;

    fn vault(dir: &tempfile::TempDir) -> std::path::PathBuf {
        let path = dir.path().join("a.vault");
        drop(VaultStore::create(&path, "real").unwrap());
        path
    }

    #[test]
    fn an_armed_duress_passphrase_opens_nothing_and_says_duress() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = vault(&dir);
        arm(&path, "duress").unwrap();
        assert!(matches!(
            VaultStore::open(&path, "duress"),
            Err(VaultError::Duress)
        ));
        assert!(VaultStore::open(&path, "real").is_ok());
        assert!(matches!(
            VaultStore::open(&path, "other"),
            Err(VaultError::WrongPassphrase)
        ));
    }

    #[test]
    fn disarming_forgets_it_and_the_header_stays_the_same_size() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = vault(&dir);
        let before = header::read(&path).unwrap();
        arm(&path, "duress").unwrap();
        let armed = header::read(&path).unwrap();
        assert_eq!(armed.salt, before.salt, "the real salt is kept");
        assert_ne!(armed.duress_salt, before.duress_salt);
        disarm(&path).unwrap();
        assert!(matches!(
            VaultStore::open(&path, "duress"),
            Err(VaultError::WrongPassphrase)
        ));
        let on_disk = std::fs::read(header::header_path(&path)).unwrap();
        assert_eq!(on_disk.len(), header::VAULT_HEADER_LEN);
        assert!(VaultStore::open(&path, "real").is_ok());
    }
}
