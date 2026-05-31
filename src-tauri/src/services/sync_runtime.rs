//! Phase 23.C — sync-handler Tauri-runtime orchestration lifted from
//! `commands/sync.rs`. Hosts `accept_pairing_code_inner` (Veilid
//! `app_call` over the existing-device route blob) and
//! `generate_pairing_qr_svg_inner` (mint session + render the deep-link
//! URI as SVG QR code).

use crate::db::DbPool;
use crate::services::cross_device_sync;
use crate::state::SharedState;
use crate::state_helpers;

pub async fn accept_pairing_code_inner(
    state: &SharedState,
    pairing_code: String,
    pairing_salt_hex: String,
    existing_device_route_blob_hex: String,
    display_name: String,
) -> Result<rekindle_types::cross_device_sync::PairingAccept, String> {
    let salt =
        hex::decode(&pairing_salt_hex).map_err(|e| format!("invalid pairing salt hex: {e}"))?;
    let route_blob = hex::decode(&existing_device_route_blob_hex)
        .map_err(|e| format!("invalid route blob hex: {e}"))?;
    let payload =
        cross_device_sync::build_pairing_payload(state, &pairing_code, &salt, &display_name)?;
    let envelope = rekindle_types::cross_device_sync::SyncEnvelope::PairingRequest(payload);
    let bytes = serde_json::to_vec(&envelope).map_err(|e| format!("encode: {e}"))?;
    let api = state_helpers::veilid_api(state).ok_or("veilid not attached")?;
    let route_id = api
        .import_remote_private_route(route_blob)
        .map_err(|e| format!("import existing device route: {e}"))?;
    let rc = state_helpers::safe_routing_context(state).ok_or("no routing context")?;
    let reply = rc
        .app_call(veilid_core::Target::RouteId(route_id), bytes)
        .await
        .map_err(|e| format!("pairing app_call failed: {e}"))?;
    let accept: rekindle_types::cross_device_sync::PairingAccept =
        serde_json::from_slice(&reply).map_err(|e| format!("pairing reply decode: {e}"))?;
    Ok(accept)
}

pub async fn generate_pairing_qr_svg_inner(
    state: &SharedState,
    pool: &DbPool,
) -> Result<crate::commands::sync::PairingQrPayload, String> {
    let session = cross_device_sync::generate_pairing_session(state, pool).await?;
    let uri = format!(
        "rekindle://pair?code={}&salt={}&route={}",
        session.pairing_code, session.pairing_salt_hex, session.existing_device_route_blob_hex,
    );
    let svg = qrcode::QrCode::with_error_correction_level(uri.as_bytes(), qrcode::EcLevel::M)
        .map_err(|e| format!("qrcode build: {e}"))?
        .render::<qrcode::render::svg::Color<'_>>()
        .min_dimensions(256, 256)
        .dark_color(qrcode::render::svg::Color("#101727"))
        .light_color(qrcode::render::svg::Color("#f8fafc"))
        .build();
    Ok(crate::commands::sync::PairingQrPayload { svg, uri, session })
}
