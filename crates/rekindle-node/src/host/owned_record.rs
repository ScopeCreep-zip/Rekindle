//! The one rule for every DHT record the identity owns with a keypair of
//! its own — its profile, friend list, account and prekey record (plan D1,
//! E2.4; audit 2026-10-20 D-5, X-4).
//!
//! - **Save.** A new record's vault entry (its key with its owner keypair,
//!   `rekindle_vault::typed::owner_keys`) is written first, then its key
//!   into the identity row. A database write that fails undoes the vault
//!   write, so an error never leaves the two naming different records.
//! - **Hold at unlock.** The vault entry and the identity row are compared:
//!   - equal: the record is reopened with the entry's keypair;
//!   - the vault names a record the row does not (a crash between the two
//!     writes, or an import, whose database is new): the row follows the
//!     vault, and that record is reopened;
//!   - the row names a record with no entry: nobody can write it any more,
//!     so a new record replaces it;
//!   - neither: a new record is created and saved.
//! - **Leases.** Every record held is held writable for the session; its
//!   lease ends with the session's record pool. The profile's and the
//!   prekey record's leases go into their `DaemonContext` slots for the
//!   writers outside the stage (a block's rotation, the prekey
//!   maintenance). A created record that could not be saved is released.

use std::future::Future;

use rekindle_db::repo::identity::OwnedRecord;
use rekindle_db::DbHandle;
use rekindle_protocol::error::ProtocolError;
use rekindle_records::lease::LeaseId;
use rekindle_records::port::RecordPort;
use rekindle_vault::typed::owner_keys::{
    forget_owner, load_owner, persist_owner, OwnerEntry, OwnerKey,
};
use rekindle_vault::{VaultHandle, VaultStore};

/// One owned record as storage holds it.
#[derive(Debug, Clone, Default)]
pub(super) struct StoredRecord {
    /// Its key in the identity row.
    pub(super) db_key: Option<String>,
    /// Its vault entry.
    pub(super) entry: Option<OwnerEntry>,
}

impl StoredRecord {
    /// Read `record` from the identity row's `db_key` and the vault.
    pub(super) fn load(
        vault: &VaultStore,
        record: OwnedRecord,
        db_key: Option<String>,
    ) -> Result<Self, String> {
        Ok(Self {
            db_key,
            entry: load_owner(vault, owner_key(record)).map_err(|e| e.to_string())?,
        })
    }

    /// The owner keypair, when the vault's entry names the row's record.
    pub(super) fn matching_keypair(&self) -> Option<&zeroize::Zeroizing<String>> {
        match (&self.db_key, &self.entry) {
            (Some(key), Some(entry)) if *key == entry.record_key => Some(&entry.keypair),
            _ => None,
        }
    }
}

/// What the unlock does with one stored record (the rule above).
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Plan<'a> {
    /// The row and the vault agree: reopen it.
    Reopen(&'a OwnerEntry),
    /// The vault names another record than the row: the row follows the
    /// vault, then reopen it.
    Repair(&'a OwnerEntry),
    /// The row names a record with no keypair: a new record replaces it.
    Replace,
    /// Nothing stored: create one.
    Create,
}

/// The rule's decision for `stored`.
pub(super) fn plan(stored: &StoredRecord) -> Plan<'_> {
    match (&stored.db_key, &stored.entry) {
        (Some(key), Some(entry)) if *key == entry.record_key => Plan::Reopen(entry),
        (_, Some(entry)) => Plan::Repair(entry),
        (Some(_), None) => Plan::Replace,
        (None, None) => Plan::Create,
    }
}

/// An owned record held writable for the session.
pub(super) struct Held {
    pub(super) key: String,
    pub(super) keypair: zeroize::Zeroizing<String>,
    pub(super) lease: LeaseId,
    /// Created now (nothing was there to reopen).
    pub(super) created: bool,
}

/// The vault entry of `record`.
pub(super) fn owner_key(record: OwnedRecord) -> OwnerKey {
    match record {
        OwnedRecord::Profile => OwnerKey::Profile,
        OwnedRecord::FriendList => OwnerKey::FriendList,
        OwnedRecord::Account => OwnerKey::Account,
        OwnedRecord::Prekeys => OwnerKey::Prekeys,
    }
}

/// What the rule needs from the identity.
pub(super) struct Owner<'a> {
    pub(super) vault: &'a VaultHandle,
    pub(super) db: &'a DbHandle,
    pub(super) public_key: &'a str,
}

impl Owner<'_> {
    /// Save a new record: its vault entry, then its key in the identity
    /// row. A failed row write puts the previous entry back.
    ///
    /// # Errors
    /// The vault is locked, or a write failed (nothing changed).
    pub(super) async fn save(
        &self,
        record: OwnedRecord,
        key: &str,
        keypair: &str,
    ) -> Result<(), String> {
        let previous = {
            let guard = self.vault.lock();
            let vault = guard.as_ref().ok_or("the vault is locked")?;
            let previous = load_owner(vault, owner_key(record)).map_err(|e| e.to_string())?;
            let entry = OwnerEntry {
                record_key: key.to_string(),
                keypair: zeroize::Zeroizing::new(keypair.to_owned()),
            };
            persist_owner(vault, owner_key(record), &entry)
                .map_err(|e| format!("owner keypair not saved: {e}"))?;
            previous
        };
        let Err(e) = self.set_key(record, key).await else {
            return Ok(());
        };
        let guard = self.vault.lock();
        if let Some(vault) = guard.as_ref() {
            let undone = match &previous {
                Some(entry) => persist_owner(vault, owner_key(record), entry),
                None => forget_owner(vault, owner_key(record)),
            };
            if let Err(undo) = undone {
                tracing::error!(error = %undo, ?record,
                    "owner entry not restored; the next unlock follows the vault");
            }
        }
        Err(format!("record key not saved: {e}"))
    }

    async fn set_key(&self, record: OwnedRecord, key: &str) -> Result<(), String> {
        let db = self.db.current().map_err(|e| e.to_string())?;
        let (pk, key) = (self.public_key.to_string(), key.to_string());
        db.call(move |conn| rekindle_db::repo::identity::set_owned_record(conn, &pk, record, &key))
            .await
            .map_err(|e| e.to_string())
    }

    /// Hold `record` for the session by the rule above: `open` reopens a
    /// stored one with `(key, keypair)`, `create` makes a new one and
    /// answers `(lease, key, keypair)`. `None` when it could not be held
    /// (logged).
    pub(super) async fn hold<O, OF, C, CF>(
        &self,
        pool: &dyn RecordPort,
        record: OwnedRecord,
        stored: &StoredRecord,
        open: O,
        create: C,
    ) -> Option<Held>
    where
        O: FnOnce(String, zeroize::Zeroizing<String>) -> OF,
        OF: Future<Output = Result<LeaseId, ProtocolError>>,
        C: FnOnce() -> CF,
        CF: Future<Output = Result<(LeaseId, String, zeroize::Zeroizing<String>), ProtocolError>>,
    {
        let entry = match plan(stored) {
            Plan::Reopen(entry) => Some(entry),
            Plan::Repair(entry) => {
                tracing::warn!(
                    ?record,
                    row_had_key = stored.db_key.is_some(),
                    "own records: the vault names another record than the identity row \
                     (an interrupted write); the row follows the vault"
                );
                if let Err(e) = self.set_key(record, &entry.record_key).await {
                    tracing::error!(error = %e, ?record, "own records: record key not repaired");
                    return None;
                }
                Some(entry)
            }
            Plan::Replace => {
                tracing::error!(
                    ?record,
                    "own records: no owner keypair is stored for the record; a new one replaces it"
                );
                None
            }
            Plan::Create => None,
        };
        if let Some(entry) = entry {
            return match open(entry.record_key.clone(), entry.keypair.clone()).await {
                Ok(lease) => Some(Held {
                    key: entry.record_key.clone(),
                    keypair: entry.keypair.clone(),
                    lease,
                    created: false,
                }),
                Err(e) => {
                    tracing::warn!(error = %e, ?record, "own records: record not reopened");
                    None
                }
            };
        }
        let (lease, key, keypair) = create()
            .await
            .map_err(|e| tracing::warn!(error = %e, ?record, "own records: record not created"))
            .ok()?;
        if let Err(e) = self.save(record, &key, &keypair).await {
            tracing::error!(error = %e, ?record, "own records: new record not saved");
            pool.release(lease).await;
            return None;
        }
        Some(Held {
            key,
            keypair,
            lease,
            created: true,
        })
    }
}

#[cfg(test)]
#[path = "owned_record_tests.rs"]
mod tests;
