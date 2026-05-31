//! Tauri commands for the cross-device sync subsystem
//! (architecture §28.4). Thin adapters over `services::cross_device_sync`.

use rekindle_secrets::sync_key::SyncKey;
use rekindle_types::cross_device_sync::{DeviceList, ReadState, SyncManifest, SyncPreferences};
use std::sync::Arc;
use tauri::State;

use crate::db::DbPool;
use crate::services::cross_device_sync;
use crate::services::cross_device_sync::PersonalSyncRecordHandle;
use crate::state::{AppState, SharedState};

#[tauri::command]
pub async fn ensure_personal_sync_record(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<String, String> {
    let handle =
        cross_device_sync::ensure_personal_sync_record(state.inner(), pool.inner()).await?;
    cross_device_sync::start_personal_sync_watch(state.inner(), pool.inner()).await?;
    Ok(handle.record_key)
}

#[tauri::command]
pub async fn start_pairing_session(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<cross_device_sync::PairingSession, String> {
    cross_device_sync::generate_pairing_session(state.inner(), pool.inner()).await
}

#[tauri::command]
pub async fn read_sync_manifest(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<Option<SyncManifest>, String> {
    let (handle, sync_key) = setup(state.inner(), pool.inner()).await?;
    cross_device_sync::read_sync_manifest(state.inner(), &handle, &sync_key).await
}

#[tauri::command]
pub async fn write_sync_manifest(
    manifest: SyncManifest,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let (handle, sync_key) = setup(state.inner(), pool.inner()).await?;
    cross_device_sync::write_sync_manifest(state.inner(), &handle, &sync_key, &manifest).await
}

#[tauri::command]
pub async fn read_sync_read_state(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<ReadState, String> {
    let (handle, sync_key) = setup(state.inner(), pool.inner()).await?;
    cross_device_sync::read_read_state(state.inner(), &handle, &sync_key).await
}

#[tauri::command]
pub async fn write_sync_read_state(
    read_state: ReadState,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<ReadState, String> {
    let (handle, sync_key) = setup(state.inner(), pool.inner()).await?;
    cross_device_sync::write_read_state(state.inner(), &handle, &sync_key, read_state).await
}

#[tauri::command]
pub async fn read_sync_preferences(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<SyncPreferences, String> {
    let (handle, sync_key) = setup(state.inner(), pool.inner()).await?;
    cross_device_sync::read_preferences(state.inner(), &handle, &sync_key).await
}

#[tauri::command]
pub async fn write_sync_preferences(
    preferences: SyncPreferences,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<SyncPreferences, String> {
    let (handle, sync_key) = setup(state.inner(), pool.inner()).await?;
    cross_device_sync::write_preferences(state.inner(), &handle, &sync_key, preferences).await
}

#[tauri::command]
pub async fn read_paired_devices(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<DeviceList, String> {
    let (handle, sync_key) = setup(state.inner(), pool.inner()).await?;
    cross_device_sync::read_device_list(state.inner(), &handle, &sync_key).await
}

#[tauri::command]
pub async fn write_paired_devices(
    devices: DeviceList,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<DeviceList, String> {
    let (handle, sync_key) = setup(state.inner(), pool.inner()).await?;
    cross_device_sync::write_device_list(state.inner(), &handle, &sync_key, devices).await
}

async fn setup(
    state: &Arc<AppState>,
    pool: &DbPool,
) -> Result<(PersonalSyncRecordHandle, SyncKey), String> {
    let handle = cross_device_sync::open_personal_sync_record(state, pool)
        .await
        .ok_or("personal sync record not initialized")?;
    let secret_opt: Option<[u8; 32]> = *state.identity_secret.lock();
    let secret = secret_opt.ok_or("no identity")?;
    Ok((handle, SyncKey::from_master_secret(&secret)))
}

/// Architecture §28.4 line 3088 — new device side.
#[tauri::command]
pub async fn accept_pairing_code(
    pairing_code: String,
    pairing_salt_hex: String,
    existing_device_route_blob_hex: String,
    display_name: String,
    state: State<'_, SharedState>,
) -> Result<rekindle_types::cross_device_sync::PairingAccept, String> {
    crate::services::sync_runtime::accept_pairing_code_inner(
        state.inner(),
        pairing_code,
        pairing_salt_hex,
        existing_device_route_blob_hex,
        display_name,
    )
    .await
}

/// Architecture §28.4 / Phase 7 W24 line 4122 — render the current
/// pairing session as an SVG QR-code string, ready for the frontend
/// to embed via `innerHTML`. Calls `start_pairing_session` to mint a
/// fresh code on every invocation, then encodes the deep-link
/// `rekindle://pair?code=<code>&salt=<hex>&route=<hex>` form so the
/// scanning device can use the same `accept_pairing_code` command
/// after parsing the URI.
///
/// Generation lives Rust-side per CLAUDE.md "all business logic in
/// Rust; frontend thin/performant" — avoids shipping a JS QR encoder.
/// SVG output (rather than PNG) keeps the result a pure string with
/// no canvas/image binding on the frontend; the WebView renders SVG
/// natively.
#[tauri::command]
pub async fn generate_pairing_qr_svg(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<PairingQrPayload, String> {
    crate::services::sync_runtime::generate_pairing_qr_svg_inner(state.inner(), pool.inner()).await
}

/// What the frontend needs in one round trip: the SVG string for
/// display, the underlying URI (so the user can copy/paste as a
/// fallback to camera scanning), and the raw session fields (so the
/// existing-device UI can show TTL countdown without re-parsing).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingQrPayload {
    /// Self-contained SVG string. Render with `<div innerHTML={...} />`.
    pub svg: String,
    /// `rekindle://pair?code=...&salt=...&route=...` — the same string
    /// encoded inside the QR. Surface as a "copy to clipboard" affordance
    /// alongside the QR so users without a working camera can still
    /// transfer the code manually.
    pub uri: String,
    pub session: cross_device_sync::PairingSession,
}
