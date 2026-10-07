//! Lifecycle of the personal cross-device sync record (architecture §28.4).
//!
//! Creates the DFLT record on first opt-in, persists the key + owner
//! keypair into the `identity` table so subsequent launches reopen it,
//! and exposes a small handle for the rest of the sync subsystem to
//! use.

use std::sync::Arc;

use rekindle_protocol::dht::schema::personal_sync_dflt_schema;

use crate::db_helpers::{db_call, db_call_or_default};
use crate::state::AppState;
use crate::state_helpers;
use rekindle_db::Db;

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

/// Returns the existing handle if the identity row already has the
/// personal sync record fields populated. `None` otherwise — the
/// caller can decide whether to call `ensure_personal_sync_record` to
/// create one.
pub async fn open_personal_sync_record(
    state: &Arc<AppState>,
    pool: &Db,
) -> Option<PersonalSyncRecordHandle> {
    let owner_key = state_helpers::current_owner_key(state).ok()?;
    let ok = owner_key.clone();
    let row: Option<(String, String, String)> = db_call_or_default(pool, move |conn| {
        rekindle_db::repo::identity::personal_sync(conn, &ok)
    })
    .await;
    row.map(
        |(record_key, owner_keypair_hex, device_id)| PersonalSyncRecordHandle {
            owner_key,
            record_key,
            owner_keypair_hex,
            device_id,
        },
    )
}

async fn persist_to_identity(
    pool: &Db,
    owner_key: &str,
    record_key: &str,
    owner_keypair_hex: &str,
    device_id: &str,
) -> Result<(), String> {
    let owner_owned = owner_key.to_string();
    let key_owned = record_key.to_string();
    let kp_owned = owner_keypair_hex.to_string();
    let did_owned = device_id.to_string();
    db_call(pool, move |conn| {
        rekindle_db::repo::identity::set_personal_sync(
            conn,
            &owner_owned,
            &key_owned,
            &kp_owned,
            &did_owned,
        )
    })
    .await
}

// `generate_device_id` lives in `rekindle_sync::cross_device::util`
// (centralised so `pairing.rs` + `record.rs` share one source of
// truth — pre-port these files each had a private copy).
use rekindle_sync::generate_device_id;
