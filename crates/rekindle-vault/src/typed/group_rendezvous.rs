//! The eight one-time writers of a group DM's rendezvous record (plan
//! decision 67 item 3), held by the group's creator, who minted them: each
//! invite gives its member its own writer only, and a grow gives a
//! newcomer a free row's. Written before the first invite is committed.

use serde::{Deserialize, Serialize};
use zeroize::Zeroize as _;

use crate::{VaultError, VaultKey, VaultStore};

/// A rendezvous record and its writers' keypair text, writer `i` for
/// slot `i`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RendezvousWriters {
    pub record_key: String,
    pub writers: Vec<String>,
}

impl std::fmt::Debug for RendezvousWriters {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RendezvousWriters")
            .field("record_key", &self.record_key)
            .field("writers", &"<redacted>")
            .finish()
    }
}

fn key(group: &str) -> VaultKey {
    VaultKey::GroupRendezvousWriters {
        group: group.to_string(),
    }
}

/// Store the writers of `group`'s rendezvous record.
///
/// # Errors
/// The entry could not be encoded or the vault write failed.
pub fn persist(
    vault: &VaultStore,
    group: &str,
    entry: &RendezvousWriters,
) -> Result<(), VaultError> {
    let mut bytes = serde_json::to_vec(entry)
        .map_err(|e| VaultError::Schema(format!("rendezvous writers: {e}")))?;
    let stored = vault.put(&key(group), &bytes);
    bytes.zeroize();
    stored
}

/// The writers of `group`'s rendezvous record, if we created it.
///
/// # Errors
/// The vault read failed, or the stored value is not an entry.
pub fn load(vault: &VaultStore, group: &str) -> Result<Option<RendezvousWriters>, VaultError> {
    vault
        .get(&key(group))?
        .map(|bytes| {
            serde_json::from_slice(&bytes)
                .map_err(|_| VaultError::Schema("rendezvous writers are not an entry".into()))
        })
        .transpose()
}

/// Forget the writers of `group`'s rendezvous record: we left the group,
/// or it is gone. Nothing held is no error.
///
/// # Errors
/// The vault delete failed.
pub fn forget(vault: &VaultStore, group: &str) -> Result<(), VaultError> {
    vault.delete(&key(group))
}
