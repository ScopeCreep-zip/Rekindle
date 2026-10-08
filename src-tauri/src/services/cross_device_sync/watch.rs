//! Personal sync record watch loop (architecture §28.4 line 3071).
//!
//! All paired devices `watch_dht_values` on subkeys 0..=3 of the
//! shared personal record. When a value change fires, the central
//! `dht_watch::handle_value_change` dispatcher delegates to
//! [`try_handle_personal_sync_change`] which decrypts the affected
//! subkey, merges into local state, and emits a `cross-device-sync`
//! Tauri event so the frontend re-hydrates.

use std::sync::Arc;

use rekindle_secrets::sync_key::{decrypt_subkey, SyncKey};
use rekindle_types::cross_device_sync::{ReadState, SUBKEY_DEVICE_LIST, SUBKEY_MANIFEST};
use veilid_core::{RecordKey, ValueSubkey};

use super::record::{open_personal_sync_record, PersonalSyncRecordHandle};
use crate::db_helpers::db_fire;
use crate::state::AppState;
use crate::state_helpers;
use rekindle_db::Db;

/// Open the personal sync record (if one exists) and request a watch
/// over all 4 active subkeys. Idempotent.
pub async fn start_personal_sync_watch(state: &Arc<AppState>, pool: &Db) -> Result<(), String> {
    let Some(handle) = open_personal_sync_record(state, pool).await else {
        return Ok(());
    };
    // The watch rides a lease the session keeps (`personal_sync_lease`),
    // so a transaction's borrow and release never cancels it.
    if state.personal_sync_lease.lock().is_some() {
        return Ok(());
    }
    let record_pool = state_helpers::record_pool(state)?;
    let key: RecordKey = handle
        .record_key
        .parse()
        .map_err(|e| format!("invalid sync record key: {e}"))?;
    let owner_kp: veilid_core::KeyPair = handle
        .owner_keypair_hex
        .parse()
        .map_err(|e| format!("invalid sync owner keypair: {e}"))?;
    let lease = record_pool
        .acquire(&key, Some(owner_kp))
        .await
        .map_err(|e| format!("open personal sync record: {e}"))?;
    if let Err(e) = record_pool
        .watch(lease, (SUBKEY_MANIFEST..=SUBKEY_DEVICE_LIST).collect())
        .await
    {
        record_pool.release(lease).await;
        return Err(format!("watch personal sync: {e}"));
    }
    let previous = state.personal_sync_lease.lock().replace(lease);
    if let Some(previous) = previous {
        record_pool.release(previous).await;
    }
    Ok(())
}

/// Returns `true` if `record_key` matches the local personal sync
/// record and the change was handled. Called from the central DHT
/// watch dispatcher.
pub async fn try_handle_personal_sync_change(
    state: &Arc<AppState>,
    pool: &Db,
    record_key: &str,
    subkeys: &[ValueSubkey],
    inline_value: Option<&[u8]>,
) -> bool {
    let Some(handle) = open_personal_sync_record(state, pool).await else {
        return false;
    };
    if handle.record_key != record_key {
        return false;
    }
    let Some(master_secret) = *state.identity_secret.lock() else {
        return false;
    };
    let sync_key = SyncKey::from_master_secret(&master_secret);

    for &subkey in subkeys {
        if !(SUBKEY_MANIFEST..=SUBKEY_DEVICE_LIST).contains(&subkey) {
            continue;
        }
        let blob = if subkeys.first() == Some(&subkey) && inline_value.is_some() {
            inline_value.map(<[u8]>::to_vec).unwrap_or_default()
        } else {
            let Ok(record_pool) = state_helpers::record_pool(state) else {
                return true;
            };
            let Ok(key) = handle.record_key.parse::<RecordKey>() else {
                return true;
            };
            match record_pool.read_once(&key, subkey, true).await {
                Ok(Some(v)) => v.data().to_vec(),
                Ok(None) | Err(_) => continue,
            }
        };
        let plaintext = match decrypt_subkey(&sync_key, subkey, &blob) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(subkey, error = %e, "personal sync subkey decrypt failed");
                continue;
            }
        };
        apply_remote_subkey(pool, &handle, subkey, &plaintext);
    }
    true
}

fn apply_remote_subkey(
    pool: &Db,
    handle: &PersonalSyncRecordHandle,
    subkey: ValueSubkey,
    plaintext: &[u8],
) {
    // Pure decode via the crate; this function owns the per-variant
    // side effect. Unknown subkeys + JSON-decode failures yield `None`
    // and are skipped.
    //
    // Read state is merged into the DB here. Preferences, the manifest and
    // the device list are read on demand by the settings commands; applying
    // a remote change to the live preferences (merged against the local
    // ones) and announcing it is plan step E6 (`preferences-changed`). The
    // previous emit went to a channel no window listened on, merged against
    // defaults rather than local values.
    let Some(decoded) = rekindle_sync::classify_remote_subkey(subkey, plaintext) else {
        return;
    };
    match decoded {
        rekindle_sync::RemoteSubkeyDecoded::ReadState(remote) => {
            merge_read_state_into_db(pool, &handle.owner_key, &handle.device_id, remote);
        }
        rekindle_sync::RemoteSubkeyDecoded::Preferences(_)
        | rekindle_sync::RemoteSubkeyDecoded::Manifest(_)
        | rekindle_sync::RemoteSubkeyDecoded::DeviceList(_) => {
            tracing::debug!(subkey, "remote personal-sync subkey changed");
        }
    }
}

/// Merge a paired device's read state into `owner_key`'s rows: the
/// read markers only advance, and onboarding finished on the other device
/// finishes here.
fn merge_read_state_into_db(pool: &Db, owner_key: &str, _device_id: &str, remote: ReadState) {
    let now = rekindle_utils::timestamp_ms_i64();
    let owner_key = owner_key.to_string();
    db_fire(pool, "merge remote read state", move |conn| {
        let tx = conn.transaction()?;
        for entry in &remote.entries {
            tx.execute(
                "INSERT INTO channel_read_state (owner_key, community_id, channel_id, last_read_lamport, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5) \
                 ON CONFLICT(owner_key, community_id, channel_id) DO UPDATE SET \
                   last_read_lamport = MAX(last_read_lamport, excluded.last_read_lamport), \
                   updated_at = excluded.updated_at",
                rusqlite::params![
                    owner_key,
                    entry.community_id,
                    entry.channel_id,
                    i64::try_from(entry.last_read_lamport).unwrap_or(i64::MAX),
                    now
                ],
            )?;
        }
        // Architecture §28.4: the per-community pseudonym is deterministic
        // per identity, so the same member row exists on every paired
        // device; marking it here stops the wizard re-showing on the
        // device that received the update.
        for (community_id, completed) in &remote.onboarding_complete {
            if *completed {
                rekindle_db::repo::members::set_my_onboarding_complete(
                    &tx,
                    &owner_key,
                    community_id,
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    });
}
