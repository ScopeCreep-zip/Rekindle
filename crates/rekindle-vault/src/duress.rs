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
#[path = "duress_tests.rs"]
mod tests;
