//! The owner keypairs of the DHT records an identity owns: its profile,
//! friend list, account and personal-sync records (plan D1: they were
//! `identity` columns before) and its prekey record (plan E2.4).
//!
//! Each entry holds the record's key with its keypair. The entry is
//! written here before the key goes into the database (D1's order), so a
//! crash between the two writes leaves an entry naming a record the
//! database does not: the unlock compares the two and repairs the database
//! from the entry (audit 2026-10-20 D-5), instead of opening the database's
//! record with another record's keypair.
//!
//! A failed write is an error, never best-effort: a record whose owner
//! keypair is lost can no longer be written.

use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

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

/// An owned record as the vault holds it: its key and its owner keypair
/// (string form).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnerEntry {
    pub record_key: String,
    pub keypair: String,
}

impl std::fmt::Debug for OwnerEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnerEntry")
            .field("record_key", &self.record_key)
            .field("keypair", &"<redacted>")
            .finish()
    }
}

/// Store `record`'s key and owner keypair, replacing the entry before.
///
/// # Errors
/// The vault write failed.
pub fn persist_owner(
    vault: &VaultStore,
    record: OwnerKey,
    entry: &OwnerEntry,
) -> Result<(), VaultError> {
    let mut bytes =
        serde_json::to_vec(entry).map_err(|e| VaultError::Schema(format!("owner entry: {e}")))?;
    let stored = vault.put(&record.vault_key(), &bytes);
    bytes.zeroize();
    stored
}

/// Store an invite's request inbox owner keypair (plan E2.5, D17), under
/// its record key.
///
/// # Errors
/// The vault write failed.
pub fn persist_inbox_owner(vault: &VaultStore, entry: &OwnerEntry) -> Result<(), VaultError> {
    let mut bytes =
        serde_json::to_vec(entry).map_err(|e| VaultError::Schema(format!("owner entry: {e}")))?;
    let key = VaultKey::RequestInboxOwnerKeypair {
        record: entry.record_key.clone(),
    };
    let stored = vault.put(&key, &bytes);
    bytes.zeroize();
    stored
}

/// Save our dm participant record's owner keypair for the conversation
/// with `conversation` (plan E2.5, MSR), before the record key is stored.
///
/// # Errors
/// The entry could not be encoded or the vault write failed.
pub fn persist_participant_owner(
    vault: &VaultStore,
    conversation: &str,
    entry: &OwnerEntry,
) -> Result<(), VaultError> {
    let mut bytes =
        serde_json::to_vec(entry).map_err(|e| VaultError::Schema(format!("owner entry: {e}")))?;
    let key = VaultKey::DmParticipantOwner {
        conversation: conversation.to_string(),
    };
    let stored = vault.put(&key, &bytes);
    bytes.zeroize();
    stored
}

/// Our dm participant record's entry for `conversation`, if one is stored.
///
/// # Errors
/// The vault read failed, or the stored value is not an entry.
pub fn load_participant_owner(
    vault: &VaultStore,
    conversation: &str,
) -> Result<Option<OwnerEntry>, VaultError> {
    vault
        .get(&VaultKey::DmParticipantOwner {
            conversation: conversation.to_string(),
        })?
        .map(|bytes| {
            serde_json::from_slice(&bytes)
                .map_err(|_| VaultError::Schema("owner entry is not an entry".into()))
        })
        .transpose()
}

/// Remove an invite's request inbox owner keypair: the invite left
/// `pending` and its inbox is dropped.
///
/// # Errors
/// The vault write failed.
pub fn forget_inbox_owner(vault: &VaultStore, record_key: &str) -> Result<(), VaultError> {
    vault.delete(&VaultKey::RequestInboxOwnerKeypair {
        record: record_key.to_string(),
    })
}

/// Remove `record`'s entry (a write that could not finish is undone).
///
/// # Errors
/// The vault write failed.
pub fn forget_owner(vault: &VaultStore, record: OwnerKey) -> Result<(), VaultError> {
    vault.delete(&record.vault_key())
}

/// `record`'s entry, if one is stored.
///
/// # Errors
/// The vault read failed, or the stored value is not an entry.
pub fn load_owner(vault: &VaultStore, record: OwnerKey) -> Result<Option<OwnerEntry>, VaultError> {
    vault
        .get(&record.vault_key())?
        .map(|bytes| {
            serde_json::from_slice(&bytes)
                .map_err(|_| VaultError::Schema("owner entry is not an entry".into()))
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(key: &str, keypair: &str) -> OwnerEntry {
        OwnerEntry {
            record_key: key.into(),
            keypair: keypair.into(),
        }
    }

    #[test]
    fn each_record_keeps_its_own_entry() {
        let dir = tempfile::TempDir::new().unwrap();
        let (vault, _) = VaultStore::create(&dir.path().join("a.vault"), "pass").unwrap();
        assert_eq!(load_owner(&vault, OwnerKey::Profile).unwrap(), None);
        persist_owner(&vault, OwnerKey::Profile, &entry("k1", "kp1")).unwrap();
        persist_owner(&vault, OwnerKey::Account, &entry("k2", "kp2")).unwrap();
        persist_owner(&vault, OwnerKey::Profile, &entry("k3", "kp3")).unwrap();
        assert_eq!(
            load_owner(&vault, OwnerKey::Profile).unwrap(),
            Some(entry("k3", "kp3"))
        );
        assert_eq!(
            load_owner(&vault, OwnerKey::Account).unwrap(),
            Some(entry("k2", "kp2"))
        );
        assert_eq!(load_owner(&vault, OwnerKey::FriendList).unwrap(), None);
        forget_owner(&vault, OwnerKey::Account).unwrap();
        assert_eq!(load_owner(&vault, OwnerKey::Account).unwrap(), None);
    }

    #[test]
    fn a_bare_keypair_is_not_an_entry() {
        let dir = tempfile::TempDir::new().unwrap();
        let (vault, _) = VaultStore::create(&dir.path().join("a.vault"), "pass").unwrap();
        vault.put(&VaultKey::ProfileOwnerKeypair, b"kp").unwrap();
        assert!(load_owner(&vault, OwnerKey::Profile).is_err());
    }

    #[test]
    fn the_debug_form_hides_the_keypair() {
        assert!(!format!("{:?}", entry("k", "secret-kp")).contains("secret-kp"));
    }
}
