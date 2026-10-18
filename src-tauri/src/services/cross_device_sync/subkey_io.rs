//! Read / write each well-known subkey of the personal sync record.
//!
//! Each subkey value on the wire is `nonce || AES-GCM-ciphertext`,
//! produced by `rekindle_secrets::sync_key::encrypt_subkey` with the
//! subkey index bound into the AAD. Reads decrypt and merge into the
//! supplied local document; writes serialize, encrypt, and `set_dht_value`.

use std::sync::Arc;

use rekindle_records::lease::LeaseId;
use rekindle_records::port::RecordPort;
use rekindle_secrets::sync_key::{decrypt_subkey, encrypt_subkey, SyncKey};
use rekindle_types::cross_device_sync::{
    DeviceList, ReadState, SyncManifest, SyncPreferences, SUBKEY_DEVICE_LIST, SUBKEY_MANIFEST,
    SUBKEY_PREFERENCES, SUBKEY_READ_STATE,
};

use super::merge::{merge_device_list, merge_preferences, merge_read_state};
use super::record::PersonalSyncRecordHandle;
use crate::state::AppState;
use crate::state_helpers;

/// Borrow the personal sync record (writable with its owner key) for one
/// read or write, then release it. The release closes the record only when
/// nothing else holds it, so the session's watch lease survives (the old
/// `close_dht_record` here cancelled that watch node-wide).
async fn borrow(
    state: &Arc<AppState>,
    handle: &PersonalSyncRecordHandle,
) -> Result<(Arc<dyn RecordPort>, LeaseId), String> {
    let pool = state_helpers::record_pool(state)?;
    let lease = pool
        .acquire(&handle.record_key, Some(&handle.owner_keypair_hex))
        .await
        .map_err(|e| format!("open personal sync record: {e}"))?;
    Ok((pool, lease))
}

/// One subkey, decrypted, on a borrow of its own.
async fn read_subkey(
    state: &Arc<AppState>,
    handle: &PersonalSyncRecordHandle,
    subkey: u32,
    sync_key: &SyncKey,
) -> Result<Option<Vec<u8>>, String> {
    let (pool, lease) = borrow(state, handle).await?;
    let read = read_decrypted_subkey(pool.as_ref(), lease, subkey, sync_key).await;
    pool.release(lease).await;
    read
}

/// Encrypt and write one subkey, on a borrow of its own.
async fn write_subkey(
    state: &Arc<AppState>,
    handle: &PersonalSyncRecordHandle,
    subkey: u32,
    sync_key: &SyncKey,
    plaintext: &[u8],
) -> Result<(), String> {
    let (pool, lease) = borrow(state, handle).await?;
    let written = write_encrypted_subkey(pool.as_ref(), lease, subkey, sync_key, plaintext).await;
    pool.release(lease).await;
    written
}

async fn read_decrypted_subkey(
    pool: &dyn RecordPort,
    lease: LeaseId,
    subkey: u32,
    sync_key: &SyncKey,
) -> Result<Option<Vec<u8>>, String> {
    let value = pool
        .get(lease, subkey, true)
        .await
        .map_err(|e| format!("get_dht_value: {e}"))?;
    let Some(value) = value else {
        return Ok(None);
    };
    let plaintext = decrypt_subkey(sync_key, subkey, &value.data)?;
    Ok(Some(plaintext))
}

async fn write_encrypted_subkey(
    pool: &dyn RecordPort,
    lease: LeaseId,
    subkey: u32,
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
    match read_subkey(state, handle, SUBKEY_MANIFEST, sync_key).await? {
        Some(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| format!("manifest decode: {e}")),
        None => Ok(None),
    }
}

pub async fn write_sync_manifest(
    state: &Arc<AppState>,
    handle: &PersonalSyncRecordHandle,
    sync_key: &SyncKey,
    manifest: &SyncManifest,
) -> Result<(), String> {
    let encoded = serde_json::to_vec(manifest).map_err(|e| format!("manifest encode: {e}"))?;
    write_subkey(state, handle, SUBKEY_MANIFEST, sync_key, &encoded).await
}

pub async fn read_read_state(
    state: &Arc<AppState>,
    handle: &PersonalSyncRecordHandle,
    sync_key: &SyncKey,
) -> Result<ReadState, String> {
    match read_subkey(state, handle, SUBKEY_READ_STATE, sync_key).await? {
        Some(bytes) => serde_json::from_slice::<ReadState>(&bytes)
            .map_err(|e| format!("read state decode: {e}")),
        None => Ok(ReadState::default()),
    }
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
    write_subkey(state, handle, SUBKEY_READ_STATE, sync_key, &encoded).await?;
    Ok(merged)
}

pub async fn read_preferences(
    state: &Arc<AppState>,
    handle: &PersonalSyncRecordHandle,
    sync_key: &SyncKey,
) -> Result<SyncPreferences, String> {
    match read_subkey(state, handle, SUBKEY_PREFERENCES, sync_key).await? {
        Some(bytes) => serde_json::from_slice::<SyncPreferences>(&bytes)
            .map_err(|e| format!("prefs decode: {e}")),
        None => Ok(SyncPreferences::default()),
    }
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
    write_subkey(state, handle, SUBKEY_PREFERENCES, sync_key, &encoded).await?;
    Ok(merged)
}

pub async fn read_device_list(
    state: &Arc<AppState>,
    handle: &PersonalSyncRecordHandle,
    sync_key: &SyncKey,
) -> Result<DeviceList, String> {
    match read_subkey(state, handle, SUBKEY_DEVICE_LIST, sync_key).await? {
        Some(bytes) => serde_json::from_slice::<DeviceList>(&bytes)
            .map_err(|e| format!("device list decode: {e}")),
        None => Ok(DeviceList::default()),
    }
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
    write_subkey(state, handle, SUBKEY_DEVICE_LIST, sync_key, &encoded).await?;
    Ok(merged)
}
