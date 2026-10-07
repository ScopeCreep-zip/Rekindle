//! Read / write each well-known subkey of the personal sync record.
//!
//! Each subkey value on the wire is `nonce || AES-GCM-ciphertext`,
//! produced by `rekindle_secrets::sync_key::encrypt_subkey` with the
//! subkey index bound into the AAD. Reads decrypt and merge into the
//! supplied local document; writes serialize, encrypt, and `set_dht_value`.

use std::sync::Arc;

use rekindle_protocol::dht::pool::RecordPool;
use rekindle_records::lease::LeaseId;
use rekindle_secrets::sync_key::{decrypt_subkey, encrypt_subkey, SyncKey};
use rekindle_types::cross_device_sync::{
    DeviceList, ReadState, SyncManifest, SyncPreferences, SUBKEY_DEVICE_LIST, SUBKEY_MANIFEST,
    SUBKEY_PREFERENCES, SUBKEY_READ_STATE,
};
use veilid_core::{KeyPair, RecordKey, ValueSubkey};

use super::merge::{merge_device_list, merge_preferences, merge_read_state};
use super::record::PersonalSyncRecordHandle;
use crate::state::AppState;
use crate::state_helpers;

/// Borrow the personal sync record (writable with its owner key) for one
/// read/write transaction, then release it. The release closes the record
/// only when nothing else holds it, so the session's watch lease survives
/// (the old `close_dht_record` here cancelled that watch node-wide).
async fn with_record<F, T>(
    state: &Arc<AppState>,
    handle: &PersonalSyncRecordHandle,
    f: F,
) -> Result<T, String>
where
    F: AsyncFnOnce(&RecordPool, LeaseId) -> Result<T, String>,
{
    let pool = state_helpers::record_pool(state)?;
    let key: RecordKey = handle
        .record_key
        .parse()
        .map_err(|e| format!("invalid sync record key: {e}"))?;
    let owner_kp: KeyPair = handle
        .owner_keypair_hex
        .parse()
        .map_err(|e| format!("invalid sync owner keypair: {e}"))?;
    let lease = pool
        .acquire(&key, Some(owner_kp))
        .await
        .map_err(|e| format!("open personal sync record: {e}"))?;
    let result = f(&pool, lease).await;
    pool.release(lease).await;
    result
}

async fn read_decrypted_subkey(
    pool: &RecordPool,
    lease: LeaseId,
    subkey: ValueSubkey,
    sync_key: &SyncKey,
) -> Result<Option<Vec<u8>>, String> {
    let value = pool
        .get(lease, subkey, true)
        .await
        .map_err(|e| format!("get_dht_value: {e}"))?;
    let Some(value) = value else {
        return Ok(None);
    };
    let plaintext = decrypt_subkey(sync_key, subkey, value.data())?;
    Ok(Some(plaintext))
}

async fn write_encrypted_subkey(
    pool: &RecordPool,
    lease: LeaseId,
    subkey: ValueSubkey,
    sync_key: &SyncKey,
    plaintext: &[u8],
) -> Result<(), String> {
    let blob = encrypt_subkey(sync_key, subkey, plaintext)?;
    let outcome = pool
        .set(lease, subkey, blob, None)
        .await
        .map_err(|e| format!("set_dht_value: {e}"))?;
    if outcome.missed() {
        return Err(format!("sync subkey {subkey} not stored ({outcome:?})"));
    }
    Ok(())
}

pub async fn read_sync_manifest(
    state: &Arc<AppState>,
    handle: &PersonalSyncRecordHandle,
    sync_key: &SyncKey,
) -> Result<Option<SyncManifest>, String> {
    with_record(
        state,
        handle,
        async |pool, lease| match read_decrypted_subkey(pool, lease, SUBKEY_MANIFEST, sync_key)
            .await?
        {
            Some(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|e| format!("manifest decode: {e}")),
            None => Ok(None),
        },
    )
    .await
}

pub async fn write_sync_manifest(
    state: &Arc<AppState>,
    handle: &PersonalSyncRecordHandle,
    sync_key: &SyncKey,
    manifest: &SyncManifest,
) -> Result<(), String> {
    let encoded = serde_json::to_vec(manifest).map_err(|e| format!("manifest encode: {e}"))?;
    with_record(state, handle, async |pool, lease| {
        write_encrypted_subkey(pool, lease, SUBKEY_MANIFEST, sync_key, &encoded).await
    })
    .await
}

pub async fn read_read_state(
    state: &Arc<AppState>,
    handle: &PersonalSyncRecordHandle,
    sync_key: &SyncKey,
) -> Result<ReadState, String> {
    with_record(
        state,
        handle,
        async |pool, lease| match read_decrypted_subkey(pool, lease, SUBKEY_READ_STATE, sync_key)
            .await?
        {
            Some(bytes) => serde_json::from_slice::<ReadState>(&bytes)
                .map_err(|e| format!("read state decode: {e}")),
            None => Ok(ReadState::default()),
        },
    )
    .await
}

pub async fn write_read_state(
    state: &Arc<AppState>,
    handle: &PersonalSyncRecordHandle,
    sync_key: &SyncKey,
    local: ReadState,
) -> Result<ReadState, String> {
    let remote = read_read_state(state, handle, sync_key)
        .await
        .unwrap_or_default();
    let merged = merge_read_state(local, remote);
    let encoded = serde_json::to_vec(&merged).map_err(|e| format!("read state encode: {e}"))?;
    with_record(state, handle, async |pool, lease| {
        write_encrypted_subkey(pool, lease, SUBKEY_READ_STATE, sync_key, &encoded).await
    })
    .await?;
    Ok(merged)
}

pub async fn read_preferences(
    state: &Arc<AppState>,
    handle: &PersonalSyncRecordHandle,
    sync_key: &SyncKey,
) -> Result<SyncPreferences, String> {
    with_record(
        state,
        handle,
        async |pool, lease| match read_decrypted_subkey(pool, lease, SUBKEY_PREFERENCES, sync_key)
            .await?
        {
            Some(bytes) => serde_json::from_slice::<SyncPreferences>(&bytes)
                .map_err(|e| format!("prefs decode: {e}")),
            None => Ok(SyncPreferences::default()),
        },
    )
    .await
}

pub async fn write_preferences(
    state: &Arc<AppState>,
    handle: &PersonalSyncRecordHandle,
    sync_key: &SyncKey,
    local: SyncPreferences,
) -> Result<SyncPreferences, String> {
    let remote = read_preferences(state, handle, sync_key)
        .await
        .unwrap_or_default();
    let merged = merge_preferences(local, remote);
    let encoded = serde_json::to_vec(&merged).map_err(|e| format!("prefs encode: {e}"))?;
    with_record(state, handle, async |pool, lease| {
        write_encrypted_subkey(pool, lease, SUBKEY_PREFERENCES, sync_key, &encoded).await
    })
    .await?;
    Ok(merged)
}

pub async fn read_device_list(
    state: &Arc<AppState>,
    handle: &PersonalSyncRecordHandle,
    sync_key: &SyncKey,
) -> Result<DeviceList, String> {
    with_record(
        state,
        handle,
        async |pool, lease| match read_decrypted_subkey(pool, lease, SUBKEY_DEVICE_LIST, sync_key)
            .await?
        {
            Some(bytes) => serde_json::from_slice::<DeviceList>(&bytes)
                .map_err(|e| format!("device list decode: {e}")),
            None => Ok(DeviceList::default()),
        },
    )
    .await
}

pub async fn write_device_list(
    state: &Arc<AppState>,
    handle: &PersonalSyncRecordHandle,
    sync_key: &SyncKey,
    local: DeviceList,
) -> Result<DeviceList, String> {
    let remote = read_device_list(state, handle, sync_key)
        .await
        .unwrap_or_default();
    let merged = merge_device_list(local, remote);
    let encoded = serde_json::to_vec(&merged).map_err(|e| format!("device list encode: {e}"))?;
    with_record(state, handle, async |pool, lease| {
        write_encrypted_subkey(pool, lease, SUBKEY_DEVICE_LIST, sync_key, &encoded).await
    })
    .await?;
    Ok(merged)
}
