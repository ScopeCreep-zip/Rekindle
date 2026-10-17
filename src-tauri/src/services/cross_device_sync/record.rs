//! Lifecycle of the personal cross-device sync record (architecture §28.4).
//!
//! Creates the DFLT record on first opt-in, persists its owner keypair
//! into the vault and its key into the `identity` row so subsequent
//! launches reopen it, and exposes a small handle for the rest of the sync
//! subsystem to use.

use std::sync::Arc;

use rekindle_protocol::dht::schema::personal_sync_dflt_schema;

use crate::db_helpers::db_call;
use crate::state::AppState;
use crate::state_helpers;
use rekindle_db::Db;
use rekindle_vault::typed::owner_keys::OwnerKey;

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
        .create(
            personal_sync_dflt_schema().map_err(|e| format!("schema: {e}"))?,
            None,
        )
        .await
        .map_err(|e| format!("personal sync record creation failed: {e}"))?;
    // The session's watch takes its own lease; the create's is not kept.
    record_pool.release(lease).await;
    let record_key = key.to_string();
    let owner_keypair_hex = owner.to_string();

    let device_id = generate_device_id();
    persist_to_identity(
        state,
        pool,
        &owner_key,
        &record_key,
        &owner_keypair_hex,
        &device_id,
    )
    .await?;

    Ok(PersonalSyncRecordHandle {
        owner_key,
        record_key,
        owner_keypair_hex,
        device_id,
    })
}

/// Returns the existing handle if the identity row has the personal sync
/// record and the vault its owner keypair. `None` otherwise — the caller
/// can decide whether to call `ensure_personal_sync_record` to create one.
pub async fn open_personal_sync_record(
    state: &Arc<AppState>,
    pool: &Db,
) -> Option<PersonalSyncRecordHandle> {
    let owner_key = state_helpers::current_owner_key(state).ok()?;
    let ok = owner_key.clone();
    let row = db_call(pool, move |conn| {
        rekindle_db::repo::identity::personal_sync(conn, &ok)
    })
    .await;
    let (record_key, device_id) = match row {
        Ok(row) => row?,
        Err(e) => {
            tracing::warn!(error = %e, "personal sync record key unreadable");
            return None;
        }
    };
    let owner_keypair_hex = match state_helpers::load_owner_keypair(state, OwnerKey::PersonalSync) {
        Ok(Some(keypair)) => keypair,
        Ok(None) => {
            tracing::warn!("personal sync record has no owner keypair in the vault");
            return None;
        }
        Err(e) => {
            tracing::warn!(error = %e, "personal sync owner keypair unreadable");
            return None;
        }
    };
    Some(PersonalSyncRecordHandle {
        owner_key,
        record_key,
        owner_keypair_hex,
        device_id,
    })
}

/// The owner keypair into the vault first, then the record key and device
/// id into the identity row, so a stored key always has its keypair.
async fn persist_to_identity(
    state: &AppState,
    pool: &Db,
    owner_key: &str,
    record_key: &str,
    owner_keypair_hex: &str,
    device_id: &str,
) -> Result<(), String> {
    state_helpers::persist_owner_keypair(state, OwnerKey::PersonalSync, owner_keypair_hex)?;
    let owner_owned = owner_key.to_string();
    let key_owned = record_key.to_string();
    let did_owned = device_id.to_string();
    db_call(pool, move |conn| {
        rekindle_db::repo::identity::set_personal_sync(conn, &owner_owned, &key_owned, &did_owned)
    })
    .await
}

// `generate_device_id` lives in `rekindle_sync::cross_device::util`
// (centralised so `pairing.rs` + `record.rs` share one source of
// truth — pre-port these files each had a private copy).
use rekindle_sync::generate_device_id;
