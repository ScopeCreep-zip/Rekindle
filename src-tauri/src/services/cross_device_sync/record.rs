//! Lifecycle of the personal cross-device sync record (architecture §28.4).
//!
//! Creates the DFLT record on first opt-in, persists the key + owner
//! keypair into the `identity` table so subsequent launches reopen it,
//! and exposes a small handle for the rest of the sync subsystem to
//! use.

use std::sync::Arc;

use rekindle_records::schema::personal_sync_dflt_schema;
use veilid_core::CRYPTO_KIND_VLD0;

use crate::db::DbPool;
use crate::db_helpers::{db_call, db_call_or_default};
use crate::state::AppState;
use crate::state_helpers;

/// Lightweight handle used by the rest of `cross_device_sync` to read
/// and write the personal record. Cloned freely.
#[derive(Clone, Debug)]
pub struct PersonalSyncRecordHandle {
    pub record_key: String,
    pub owner_keypair_hex: String,
    pub device_id: String,
}

/// Idempotent: returns the existing handle if one is already on disk,
/// otherwise creates a fresh personal DFLT record + owner keypair and
/// stores them. Caller must be logged in.
pub async fn ensure_personal_sync_record(
    state: &Arc<AppState>,
    pool: &DbPool,
) -> Result<PersonalSyncRecordHandle, String> {
    if let Some(handle) = open_personal_sync_record(state, pool).await {
        return Ok(handle);
    }

    let owner_key = state_helpers::current_owner_key(state)?;
    let rc = state_helpers::routing_context(state).ok_or("not attached")?;
    let desc = rc
        .create_dht_record(
            CRYPTO_KIND_VLD0,
            personal_sync_dflt_schema().map_err(|e| format!("schema: {e}"))?,
            None,
        )
        .await
        .map_err(|e| format!("personal sync record creation failed: {e}"))?;

    let record_key = desc.key().to_string();
    let owner_keypair_hex = desc
        .owner_secret()
        .map(|s| {
            let kp = veilid_core::KeyPair::new_from_parts(desc.owner().clone(), s.value());
            kp.to_string()
        })
        .ok_or("personal sync record missing owner secret")?;

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
    pool: &DbPool,
) -> Option<PersonalSyncRecordHandle> {
    let owner_key = state_helpers::current_owner_key(state).ok()?;
    let row: Option<(String, String, String)> = db_call_or_default(pool, move |conn| {
        let mut stmt = conn.prepare(
            "SELECT personal_sync_record_key, personal_sync_owner_keypair, device_id \
               FROM identity WHERE public_key = ?1",
        )?;
        let mut rows = stmt.query(rusqlite::params![owner_key])?;
        if let Some(row) = rows.next()? {
            let key: Option<String> = row.get(0)?;
            let kp: Option<String> = row.get(1)?;
            let id: Option<String> = row.get(2)?;
            if let (Some(k), Some(p), Some(d)) = (key, kp, id) {
                if !k.is_empty() && !p.is_empty() && !d.is_empty() {
                    return Ok(Some((k, p, d)));
                }
            }
        }
        Ok(None)
    })
    .await;
    row.map(|(record_key, owner_keypair_hex, device_id)| PersonalSyncRecordHandle {
        record_key,
        owner_keypair_hex,
        device_id,
    })
}

async fn persist_to_identity(
    pool: &DbPool,
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
        conn.execute(
            "UPDATE identity SET personal_sync_record_key = ?1, \
                 personal_sync_owner_keypair = ?2, device_id = ?3 \
              WHERE public_key = ?4",
            rusqlite::params![key_owned, kp_owned, did_owned, owner_owned],
        )?;
        Ok(())
    })
    .await
}

fn generate_device_id() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

