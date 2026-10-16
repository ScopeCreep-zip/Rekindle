//! The owner keypairs of the DM logs the daemon shares with its friends
//! (the friend-inbox protocol, until E2.1), stored as the keypair's bytes.
//!
//! A failed write is an error: a log whose keypair is lost can no longer
//! be written.

use zeroize::Zeroizing;

use crate::{VaultError, VaultKey, VaultStore};

fn key(peer: &str) -> VaultKey {
    VaultKey::DmLogKeypair {
        peer: peer.to_string(),
    }
}

/// Store the keypair of the DM log shared with `peer`.
///
/// # Errors
/// The vault write failed.
pub fn persist_dm_log_keypair(
    vault: &VaultStore,
    peer: &str,
    keypair: &[u8],
) -> Result<(), VaultError> {
    vault.put(&key(peer), keypair)
}

/// The keypair of the DM log shared with `peer`, if one is stored.
///
/// # Errors
/// The vault read failed.
pub fn load_dm_log_keypair(
    vault: &VaultStore,
    peer: &str,
) -> Result<Option<Zeroizing<Vec<u8>>>, VaultError> {
    vault.get(&key(peer))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_keypair_is_kept_per_peer() {
        let dir = tempfile::TempDir::new().unwrap();
        let (vault, _) = VaultStore::create(&dir.path().join("a.vault"), "pass").unwrap();
        persist_dm_log_keypair(&vault, "bob", &[1, 2]).unwrap();
        assert_eq!(
            load_dm_log_keypair(&vault, "bob")
                .unwrap()
                .as_deref()
                .map(Vec::as_slice),
            Some(&[1u8, 2][..])
        );
        assert!(load_dm_log_keypair(&vault, "carol").unwrap().is_none());
    }
}
