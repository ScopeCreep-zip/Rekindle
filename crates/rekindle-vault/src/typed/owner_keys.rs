//! The owner keypairs of the DHT records an identity owns: its profile,
//! friend list, account and personal-sync records (plan D1: they were
//! `identity` columns before) and its prekey record (plan E2.4). Each is stored as the keypair's string form.
//!
//! A failed write is an error, never best-effort: a record whose owner
//! keypair is lost can no longer be written.

use crate::{VaultError, VaultKey, VaultStore};

/// A record the identity owns with a keypair of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerKey {
    Profile,
    FriendList,
    Account,
    PersonalSync,
    Prekeys,
}

impl OwnerKey {
    fn vault_key(self) -> VaultKey {
        match self {
            Self::Profile => VaultKey::ProfileOwnerKeypair,
            Self::FriendList => VaultKey::FriendListOwnerKeypair,
            Self::Account => VaultKey::AccountOwnerKeypair,
            Self::PersonalSync => VaultKey::PersonalSyncOwnerKeypair,
            Self::Prekeys => VaultKey::PrekeyRecordOwnerKeypair,
        }
    }
}

/// Store `record`'s owner keypair, replacing the one before.
///
/// # Errors
/// The vault write failed.
pub fn persist_owner_keypair(
    vault: &VaultStore,
    record: OwnerKey,
    keypair: &str,
) -> Result<(), VaultError> {
    vault.put(&record.vault_key(), keypair.as_bytes())
}

/// `record`'s owner keypair, if one is stored.
///
/// # Errors
/// The vault read failed, or the stored value is not UTF-8.
pub fn load_owner_keypair(
    vault: &VaultStore,
    record: OwnerKey,
) -> Result<Option<String>, VaultError> {
    vault
        .get(&record.vault_key())?
        .map(|bytes| {
            String::from_utf8(bytes.to_vec())
                .map_err(|_| VaultError::Schema("owner keypair is not UTF-8".into()))
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_record_keeps_its_own_keypair() {
        let dir = tempfile::TempDir::new().unwrap();
        let (vault, _) = VaultStore::create(&dir.path().join("a.vault"), "pass").unwrap();
        assert_eq!(load_owner_keypair(&vault, OwnerKey::Profile).unwrap(), None);
        persist_owner_keypair(&vault, OwnerKey::Profile, "kp1").unwrap();
        persist_owner_keypair(&vault, OwnerKey::Account, "kp2").unwrap();
        persist_owner_keypair(&vault, OwnerKey::Profile, "kp3").unwrap();
        assert_eq!(
            load_owner_keypair(&vault, OwnerKey::Profile)
                .unwrap()
                .as_deref(),
            Some("kp3")
        );
        assert_eq!(
            load_owner_keypair(&vault, OwnerKey::Account)
                .unwrap()
                .as_deref(),
            Some("kp2")
        );
        assert_eq!(
            load_owner_keypair(&vault, OwnerKey::FriendList).unwrap(),
            None
        );
    }
}
