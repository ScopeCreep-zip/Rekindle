//! Tauri commands for the cross-device sync subsystem
//! (architecture §28.4). Thin adapters over `services::cross_device_sync`.

use rekindle_secrets::sync_key::SyncKey;
use rekindle_types::cross_device_sync::{
    DeviceList, ReadState, SyncManifest, SyncPreferences,
};
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
    let handle = cross_device_sync::ensure_personal_sync_record(state.inner(), pool.inner())
        .await?;
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

/// Architecture §28.4 line 3088 — new device side. Builds a
/// `PairingPayload` with the master secret wrapped under the pairing
/// key derived from `(code, salt)`, then dials the existing device
/// over `app_call` using the route blob shipped in the QR code.
#[tauri::command]
pub async fn accept_pairing_code(
    pairing_code: String,
    pairing_salt_hex: String,
    existing_device_route_blob_hex: String,
    display_name: String,
    state: State<'_, SharedState>,
) -> Result<rekindle_types::cross_device_sync::PairingAccept, String> {
    let salt = hex::decode(&pairing_salt_hex)
        .map_err(|e| format!("invalid pairing salt hex: {e}"))?;
    let route_blob = hex::decode(&existing_device_route_blob_hex)
        .map_err(|e| format!("invalid route blob hex: {e}"))?;
    let payload = cross_device_sync::build_pairing_payload(
        state.inner(),
        &pairing_code,
        &salt,
        &display_name,
    )?;
    let envelope = rekindle_types::cross_device_sync::SyncEnvelope::PairingRequest(payload);
    let bytes = serde_json::to_vec(&envelope).map_err(|e| format!("encode: {e}"))?;
    let api = crate::state_helpers::veilid_api(state.inner())
        .ok_or("veilid not attached")?;
    let route_id = api
        .import_remote_private_route(route_blob)
        .map_err(|e| format!("import existing device route: {e}"))?;
    let rc = crate::state_helpers::safe_routing_context(state.inner())
        .ok_or("no routing context")?;
    let reply = rc
        .app_call(veilid_core::Target::RouteId(route_id), bytes)
        .await
        .map_err(|e| format!("pairing app_call failed: {e}"))?;
    let accept: rekindle_types::cross_device_sync::PairingAccept =
        serde_json::from_slice(&reply).map_err(|e| format!("pairing reply decode: {e}"))?;
    Ok(accept)
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
    let session = cross_device_sync::generate_pairing_session(state.inner(), pool.inner()).await?;
    // The pairing code is base32 (URL-safe alphabet) and the other two
    // fields are hex; none need percent-encoding.
    let uri = format!(
        "rekindle://pair?code={}&salt={}&route={}",
        session.pairing_code,
        session.pairing_salt_hex,
        session.existing_device_route_blob_hex,
    );
    let svg = qrcode::QrCode::with_error_correction_level(uri.as_bytes(), qrcode::EcLevel::M)
        .map_err(|e| format!("qrcode build: {e}"))?
        .render::<qrcode::render::svg::Color<'_>>()
        .min_dimensions(256, 256)
        .dark_color(qrcode::render::svg::Color("#101727"))
        .light_color(qrcode::render::svg::Color("#f8fafc"))
        .build();
    Ok(PairingQrPayload {
        svg,
        uri,
        session,
    })
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
