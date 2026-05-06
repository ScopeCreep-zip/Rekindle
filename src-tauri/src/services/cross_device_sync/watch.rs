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
use rekindle_types::cross_device_sync::{
    DeviceList, ReadState, SyncManifest, SyncPreferences, SUBKEY_DEVICE_LIST, SUBKEY_MANIFEST,
    SUBKEY_PREFERENCES, SUBKEY_READ_STATE,
};
use tauri::{AppHandle, Emitter as _};
use veilid_core::{RecordKey, ValueSubkey, ValueSubkeyRangeSet};

use super::merge::{merge_device_list, merge_manifest, merge_preferences};
use super::record::{open_personal_sync_record, PersonalSyncRecordHandle};
use crate::db::DbPool;
use crate::db_helpers::db_fire;
use crate::state::AppState;
use crate::state_helpers;

/// Open the personal sync record (if one exists) and request a watch
/// over all 4 active subkeys. Idempotent.
pub async fn start_personal_sync_watch(
    state: &Arc<AppState>,
    pool: &DbPool,
) -> Result<(), String> {
    let Some(handle) = open_personal_sync_record(state, pool).await else {
        return Ok(());
    };
    let rc = state_helpers::safe_routing_context(state).ok_or("not attached")?;
    let key: RecordKey = handle
        .record_key
        .parse()
        .map_err(|e| format!("invalid sync record key: {e}"))?;
    let owner_kp: veilid_core::KeyPair = handle
        .owner_keypair_hex
        .parse()
        .map_err(|e| format!("invalid sync owner keypair: {e}"))?;
    let _ = rc
        .open_dht_record(key.clone(), Some(owner_kp))
        .await
        .map_err(|e| format!("open personal sync record: {e}"))?;
    let mut subkeys = ValueSubkeyRangeSet::new();
    for sk in SUBKEY_MANIFEST..=SUBKEY_DEVICE_LIST {
        subkeys = subkeys.union(&ValueSubkeyRangeSet::single(sk));
    }
    let _ = rc
        .watch_dht_values(key, Some(subkeys), None, None)
        .await
        .map_err(|e| format!("watch personal sync: {e}"))?;
    Ok(())
}

/// Returns `true` if `record_key` matches the local personal sync
/// record and the change was handled. Called from the central DHT
/// watch dispatcher.
pub async fn try_handle_personal_sync_change(
    app_handle: &AppHandle,
    state: &Arc<AppState>,
    pool: &DbPool,
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
            let Some(rc) = state_helpers::safe_routing_context(state) else {
                return true;
            };
            let Ok(key) = handle.record_key.parse::<RecordKey>() else {
                return true;
            };
            match rc.get_dht_value(key, subkey, true).await {
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
        apply_remote_subkey(app_handle, pool, &handle, subkey, &plaintext);
    }
    true
}

fn apply_remote_subkey(
    app_handle: &AppHandle,
    pool: &DbPool,
    handle: &PersonalSyncRecordHandle,
    subkey: ValueSubkey,
    plaintext: &[u8],
) {
    match subkey {
        SUBKEY_READ_STATE => {
            if let Ok(remote) = serde_json::from_slice::<ReadState>(plaintext) {
                merge_read_state_into_db(pool, &handle.device_id, remote);
            }
        }
        SUBKEY_PREFERENCES => {
            if let Ok(remote) = serde_json::from_slice::<SyncPreferences>(plaintext) {
                let _ = app_handle.emit(
                    "cross-device-sync",
                    SyncEvent::Preferences(merge_preferences(SyncPreferences::default(), remote)),
                );
            }
        }
        SUBKEY_MANIFEST => {
            if let Ok(remote) = serde_json::from_slice::<SyncManifest>(plaintext) {
                let _ = app_handle.emit(
                    "cross-device-sync",
                    SyncEvent::Manifest(merge_manifest(SyncManifest::default(), remote)),
                );
            }
        }
        SUBKEY_DEVICE_LIST => {
            if let Ok(remote) = serde_json::from_slice::<DeviceList>(plaintext) {
                let _ = app_handle.emit(
                    "cross-device-sync",
                    SyncEvent::DeviceList(merge_device_list(DeviceList::default(), remote)),
                );
            }
        }
        _ => {}
    }
}

fn merge_read_state_into_db(pool: &DbPool, _device_id: &str, remote: ReadState) {
    let now = rekindle_utils::timestamp_ms_i64();
    db_fire(pool, "merge remote read state", move |conn| {
        let tx = conn.transaction()?;
        for entry in &remote.entries {
            tx.execute(
                "INSERT INTO channel_read_state (owner_key, community_id, channel_id, last_read_lamport, updated_at) \
                 SELECT public_key, ?1, ?2, ?3, ?4 FROM identity LIMIT 1 \
                 ON CONFLICT(owner_key, community_id, channel_id) DO UPDATE SET \
                   last_read_lamport = MAX(last_read_lamport, excluded.last_read_lamport), \
                   updated_at = excluded.updated_at",
                rusqlite::params![
                    entry.community_id,
                    entry.channel_id,
                    i64::try_from(entry.last_read_lamport).unwrap_or(i64::MAX),
                    now
                ],
            )?;
        }
        // Architecture §28.4 — apply the SMPL `onboarding_complete` map
        // to the local SQLite mirror. The per-community pseudonym is
        // deterministic per identity, so the same `(owner_key,
        // community_id, my_pseudonym_key)` row exists on every paired
        // device; flipping it to 1 here is what stops the wizard from
        // re-showing on the device that received the SMPL update.
        for (community_id, completed) in &remote.onboarding_complete {
            if !*completed {
                continue;
            }
            tx.execute(
                "UPDATE community_members \
                 SET onboarding_complete = 1 \
                 WHERE community_id = ?1 \
                   AND owner_key = (SELECT public_key FROM identity LIMIT 1) \
                   AND pseudonym_key = ( \
                     SELECT my_pseudonym_key FROM communities \
                      WHERE id = ?1 AND owner_key = (SELECT public_key FROM identity LIMIT 1) \
                   )",
                rusqlite::params![community_id],
            )?;
        }
        tx.commit()?;
        Ok(())
    });
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(tag = "type", content = "data", rename_all = "camelCase")]
enum SyncEvent {
    Manifest(SyncManifest),
    Preferences(SyncPreferences),
    DeviceList(DeviceList),
}
