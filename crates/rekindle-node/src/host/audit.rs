//! The unlocked identity's audit chain, for both hosts (plan D1): the I/O
//! around `rekindle_audit`'s restore and verify — the entries in the
//! identity's database (`rekindle_db::repo::audit`), the MAC key and tail
//! anchor in its vault (`rekindle_vault::typed::audit`).
//!
//! Appends are best-effort and never block the action they record: the
//! chain advances in memory, the row is queued in order on the database
//! thread, and the tail anchor is written to the vault. Nothing is written
//! while locked: no identity database is open, and an unauthenticated
//! caller must not write into one.

use parking_lot::Mutex;
use rekindle_audit::{AuditChain, AuditEntry, AuditKind, AuditRecord, Restored, Verified};
use rekindle_db::Db;
use rekindle_vault::typed::audit::{
    load_audit_tail, load_or_create_audit_mac_key, persist_audit_tail,
};
use rekindle_vault::VaultHandle;

/// The host's slot for the unlocked identity's chain (`None` while locked).
pub type ChainSlot = Mutex<Option<AuditChain>>;

/// Open `owner`'s chain into `slot` (unlock): it continues from the stored
/// tail, or from the vault's anchor when rows were removed after it.
///
/// # Errors
/// The vault is locked, or a store could not be read.
pub async fn restore(
    slot: &ChainSlot,
    db: &Db,
    vault: &VaultHandle,
    owner: &str,
) -> Result<Restored, String> {
    let o = owner.to_string();
    let stored = db
        .call(move |conn| rekindle_db::repo::audit::load_tail(conn, &o))
        .await
        .map_err(|e| format!("audit chain tail: {e}"))?;
    let (key, anchor) = {
        let guard = vault.lock();
        let vault = guard.as_ref().ok_or("the vault is locked")?;
        (load_or_create_audit_mac_key(vault)?, load_audit_tail(vault))
    };
    let (chain, restored) = rekindle_audit::restore(zeroize::Zeroizing::new(key), anchor, stored);
    *slot.lock() = Some(chain);
    Ok(restored)
}

/// Append one entry for `owner`. A no-op while locked.
pub fn append(
    slot: &ChainSlot,
    db: &Db,
    vault: &VaultHandle,
    owner: &str,
    kind: AuditKind,
    payload: serde_json::Value,
) {
    let record = AuditRecord {
        at_ms: rekindle_utils::timestamp_ms_i64(),
        actor_pub: owner.to_string(),
        kind,
        payload,
    };
    let entry = {
        let mut guard = slot.lock();
        let Some(chain) = guard.as_mut() else {
            return;
        };
        match chain.append(record) {
            Ok(entry) => entry,
            Err(e) => {
                tracing::warn!(error = %e, "audit entry not serialized");
                return;
            }
        }
    };
    let (owner, row) = (owner.to_string(), entry.clone());
    db.fire("audit entry", move |conn| {
        rekindle_db::repo::audit::insert_entry(conn, &owner, &row)
    });
    // The anchor in the vault, a separate encrypted file: whoever can edit
    // the database cannot move it with a truncation.
    if let Some(vault) = vault.lock().as_ref() {
        if let Err(e) = persist_audit_tail(vault, entry.cursor, &entry.mac) {
            tracing::warn!(cursor = entry.cursor, error = %e, "audit tail anchor not written");
        }
    }
}

/// Verify `owner`'s chain end to end.
pub async fn verify(slot: &ChainSlot, db: &Db, owner: &str) -> Verified {
    let o = owner.to_string();
    let entries = match db
        .call(move |conn| rekindle_db::repo::audit::load_all(conn, &o))
        .await
    {
        Ok(entries) => entries,
        Err(e) => {
            return Verified::Broken {
                length: 0,
                at: None,
                detail: format!("audit entries unreadable: {e}"),
            }
        }
    };
    let guard = slot.lock();
    match guard.as_ref() {
        Some(chain) => rekindle_audit::verify(chain, &entries),
        None => Verified::Broken {
            length: entries.len() as u64,
            at: None,
            detail: "no identity is unlocked".into(),
        },
    }
}

/// `owner`'s entries after `since`.
///
/// # Errors
/// The database could not be read.
pub async fn export(db: &Db, owner: &str, since: u64) -> Result<Vec<AuditEntry>, String> {
    let owner = owner.to_string();
    db.call(move |conn| rekindle_db::repo::audit::load_since(conn, &owner, since))
        .await
        .map_err(|e| format!("audit entries: {e}"))
}

/// Record the wrong passphrases an unlock cleared, as one entry.
pub fn record_unlock_failures(
    slot: &ChainSlot,
    db: &Db,
    vault: &VaultHandle,
    owner: &str,
    count: u32,
) {
    if count > 0 {
        append(
            slot,
            db,
            vault,
            owner,
            AuditKind::UnlockFailures,
            serde_json::json!({ "count": count }),
        );
    }
}

/// Close the chain (lock).
pub fn close(slot: &ChainSlot) {
    slot.lock().take();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::fixture::Fixture;

    #[tokio::test]
    async fn appended_entries_verify_and_survive_a_relock() {
        let f = Fixture::new();
        let made = crate::host::identity::create(&f.root, &f.stores(), None, "pass", 1)
            .await
            .unwrap();
        let owner = made.public_key.clone();
        drop(made);
        let db = f.db.current().unwrap();
        let slot = ChainSlot::new(None);
        assert_eq!(
            restore(&slot, &db, &f.vault, &owner).await.unwrap(),
            Restored::Intact
        );
        append(
            &slot,
            &db,
            &f.vault,
            &owner,
            AuditKind::VaultUnlocked,
            serde_json::json!({}),
        );
        record_unlock_failures(&slot, &db, &f.vault, &owner, 3);
        record_unlock_failures(&slot, &db, &f.vault, &owner, 0);
        assert_eq!(
            verify(&slot, &db, &owner).await,
            Verified::Intact { length: 2 }
        );

        close(&slot);
        assert_eq!(
            restore(&slot, &db, &f.vault, &owner).await.unwrap(),
            Restored::Intact
        );
        append(
            &slot,
            &db,
            &f.vault,
            &owner,
            AuditKind::Request,
            serde_json::json!({}),
        );
        assert_eq!(
            verify(&slot, &db, &owner).await,
            Verified::Intact { length: 3 }
        );
        assert_eq!(export(&db, &owner, 1).await.unwrap().len(), 2);
    }
}
