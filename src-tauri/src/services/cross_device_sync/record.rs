//! Lifecycle of the personal cross-device sync record (architecture §28.4).
//!
//! Creates the DFLT record on first opt-in, persists its vault entry (key
//! and owner keypair) and then its key into the `identity` row so
//! subsequent launches reopen it, and exposes a small handle for the rest
//! of the sync subsystem to use. The record names this device by the id
//! the own-records stage minted (`identity.device_id`); it never mints or
//! replaces one of its own (audit 2026-10-20 D-6).

use std::sync::Arc;

use crate::db_helpers::db_call;
use crate::state::AppState;
use crate::state_helpers;
use rekindle_db::Db;
use rekindle_vault::typed::owner_keys::{OwnerEntry, OwnerKey};

/// Lightweight handle used by the rest of `cross_device_sync` to read
/// and write the personal record. Cloned freely.
#[derive(Clone, Debug)]
pub struct PersonalSyncRecordHandle {
    /// The identity whose record this is; remote changes apply to its rows.
    pub owner_key: String,
    pub record_key: String,
    pub owner_keypair_hex: String,
    pub device_id: String,
}

/// Idempotent: returns the existing handle if one is already on disk,
/// otherwise creates a fresh personal DFLT record + owner keypair and
/// stores them. Caller must be logged in.
pub async fn ensure_personal_sync_record(
    state: &Arc<AppState>,
    pool: &Db,
) -> Result<PersonalSyncRecordHandle, String> {
    if let Some(handle) = open_personal_sync_record(state, pool).await {
        return Ok(handle);
    }

    let owner_key = state_helpers::current_owner_key(state)?;
    let record_pool = state_helpers::record_pool(state)?;
    let (lease, key, owner) = record_pool
        .create(&rekindle_records::schema::personal_sync(), None)
        .await
        .map_err(|e| format!("personal sync record creation failed: {e}"))?;
    // The session's watch takes its own lease; the create's is not kept.
    record_pool.release(lease).await;
    let record_key = key;
    let owner_keypair_hex = owner;

    let device_id = persist_to_identity(
        state,
        pool,
        &owner_key,
        &record_key,
        &owner_keypair_hex,
        &generate_device_id(),
    )
    .await?;

    Ok(PersonalSyncRecordHandle {
        owner_key,
        record_key,
        owner_keypair_hex,
        device_id,
    })
}

/// Returns the existing handle if the vault holds the personal sync
/// record's entry and the identity row its key. A vault entry naming
/// another record than the row (a crash between the two writes) is the
/// one kept: the row follows it, as the own-records stage does for the
/// other owned records. `None` otherwise — the caller can decide whether
/// to call `ensure_personal_sync_record` to create one.
pub async fn open_personal_sync_record(
    state: &Arc<AppState>,
    pool: &Db,
) -> Option<PersonalSyncRecordHandle> {
    let owner_key = state_helpers::current_owner_key(state).ok()?;
    let entry = match state_helpers::load_owner(state, OwnerKey::PersonalSync) {
        Ok(Some(entry)) => entry,
        Ok(None) => return None,
        Err(e) => {
            tracing::warn!(error = %e, "personal sync owner keypair unreadable");
            return None;
        }
    };
    let ok = owner_key.clone();
    let row = db_call(pool, move |conn| {
        rekindle_db::repo::identity::personal_sync(conn, &ok)
    })
    .await;
    let device_id = match row {
        Ok(Some((record_key, device_id))) if record_key == entry.record_key => device_id,
        Ok(_) => {
            tracing::warn!("personal sync: the vault names another record than the identity row; the row follows the vault");
            let (ok, key) = (owner_key.clone(), entry.record_key.clone());
            let minted = generate_device_id();
            match db_call(pool, move |conn| {
                rekindle_db::repo::identity::set_personal_sync(conn, &ok, &key, &minted)
            })
            .await
            {
                Ok(device_id) => device_id,
                Err(e) => {
                    tracing::warn!(error = %e, "personal sync record key not repaired");
                    return None;
                }
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "personal sync record key unreadable");
            return None;
        }
    };
    Some(PersonalSyncRecordHandle {
        owner_key,
        record_key: entry.record_key,
        owner_keypair_hex: entry.keypair,
        device_id,
    })
}

/// The vault entry (key and owner keypair) first, then the record key into
/// the identity row; answers this device's id (the one held, or `minted`
/// when the row has none), so a stored key always has its keypair and the
/// device id is never replaced.
async fn persist_to_identity(
    state: &AppState,
    pool: &Db,
    owner_key: &str,
    record_key: &str,
    owner_keypair_hex: &str,
    minted_device_id: &str,
) -> Result<String, String> {
    state_helpers::persist_owner(
        state,
        OwnerKey::PersonalSync,
        &OwnerEntry {
            record_key: record_key.to_string(),
            keypair: owner_keypair_hex.to_string(),
        },
    )?;
    let owner_owned = owner_key.to_string();
    let key_owned = record_key.to_string();
    let minted = minted_device_id.to_string();
    db_call(pool, move |conn| {
        rekindle_db::repo::identity::set_personal_sync(conn, &owner_owned, &key_owned, &minted)
    })
    .await
}

// `generate_device_id` lives in `rekindle_sync::cross_device::util`
// (centralised so `pairing.rs` + `record.rs` share one source of
// truth — pre-port these files each had a private copy).
use rekindle_sync::generate_device_id;
