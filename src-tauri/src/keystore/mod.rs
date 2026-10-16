//! The desktop's identity vaults: one `VaultStore` per identity, opened at
//! login into the shared `rekindle_vault::VaultHandle`. The typed helpers
//! for each kind of secret are `rekindle_vault::typed` (plan C5.4).

pub use rekindle_vault::typed::audit::{
    load_audit_tail, load_or_create_audit_mac_key, persist_audit_tail,
};
pub use rekindle_vault::typed::community_keys::{
    delete_registry_keypair, delete_slot_keypair, delete_slot_seed, load_registry_keypair,
    load_slot_keypair, load_slot_seed, persist_registry_keypair, persist_slot_keypair,
    persist_slot_seed,
};
pub use rekindle_vault::typed::mek::{
    delete_scope_meks, load_latest_mek, load_mek_generation, persist_mek,
};
pub use rekindle_vault::typed::signal::{
    delete_signal_pq_one_time, delete_signal_prekey, delete_signal_session,
    list_signal_pq_one_time_ids, list_signal_prekey_ids, list_signal_sessions,
    load_signal_pq_secret, load_signal_prekey, load_signal_session, load_signal_signed_prekey,
    load_trusted_identity, persist_signal_pq_secret, persist_signal_prekey, persist_signal_session,
    persist_signal_signed_prekey, persist_trusted_identity,
};

use std::path::Path;

use rekindle_crypto::CryptoError;
use rekindle_vault::VaultStore;

/// Create the vault `{vault_dir}/{public_key_hex}.vault` (and its `.hdr`)
/// for a new identity.
pub fn create_identity_vault(
    vault_dir: &Path,
    public_key_hex: &str,
    passphrase: &str,
) -> Result<VaultStore, CryptoError> {
    let path = vault_dir.join(format!("{public_key_hex}.vault"));
    VaultStore::create(&path, passphrase)
        .map(|(vault, _app_db_key)| vault)
        .map_err(map_vault_error)
}

/// Open an existing identity's vault. Nothing is created.
pub fn open_identity_vault(
    vault_dir: &Path,
    public_key_hex: &str,
    passphrase: &str,
) -> Result<VaultStore, CryptoError> {
    let path = vault_dir.join(format!("{public_key_hex}.vault"));
    VaultStore::open(&path, passphrase)
        .map(|(vault, _app_db_key)| vault)
        .map_err(map_vault_error)
}

/// Delete an identity's vault and its header. Missing files are tolerated.
pub fn delete_identity_vault(vault_dir: &Path, public_key_hex: &str) -> Result<(), std::io::Error> {
    let path = vault_dir.join(format!("{public_key_hex}.vault"));
    VaultStore::delete_files(&path).map_err(|e| match e {
        rekindle_vault::VaultError::Io(io) => io,
        other => std::io::Error::other(other.to_string()),
    })
}

/// The test vault for identity "t" in `dir`: created on first use and
/// reopened after, as the tests reopen a vault to simulate a restart.
#[cfg(test)]
pub(crate) fn test_keystore(dir: &Path, pass: &str) -> Result<VaultStore, CryptoError> {
    if dir.join("t.vault").exists() {
        open_identity_vault(dir, "t", pass)
    } else {
        create_identity_vault(dir, "t", pass)
    }
}

fn map_vault_error(e: rekindle_vault::VaultError) -> CryptoError {
    match e {
        rekindle_vault::VaultError::WrongPassphrase => CryptoError::WrongPassphrase,
        other => CryptoError::storage(format!("vault open: {other}")),
    }
}

/// Map a keystore initialization / unlock error to a user-facing string:
/// a passphrase that does not open the vault is "Wrong passphrase"
/// (`CryptoError::WrongPassphrase`, typed at the vault's keying phase);
/// anything else passes its own text through.
pub fn map_stronghold_error(e: &rekindle_crypto::CryptoError) -> String {
    match e {
        rekindle_crypto::CryptoError::WrongPassphrase => {
            "Wrong passphrase — unable to unlock keystore".to_string()
        }
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rekindle_vault::VaultKey;
    use tempfile::TempDir;

    #[test]
    fn a_vault_persists_across_reopen_and_refuses_a_wrong_passphrase() {
        let dir = TempDir::new().unwrap();
        {
            let ks = test_keystore(dir.path(), "pass123").unwrap();
            ks.put(&VaultKey::IdentityEd25519, &[7u8; 32]).unwrap();
        }
        let ks = test_keystore(dir.path(), "pass123").unwrap();
        assert_eq!(
            &*ks.get(&VaultKey::IdentityEd25519).unwrap().unwrap(),
            &[7u8; 32]
        );
        drop(ks);
        assert!(matches!(
            test_keystore(dir.path(), "wrong"),
            Err(CryptoError::WrongPassphrase)
        ));
        delete_identity_vault(dir.path(), "t").unwrap();
        assert!(!dir.path().join("t.vault").exists());
    }
}
