use tauri::State;

use crate::db::DbPool;
use crate::state::SharedState;
use rekindle_types::permissions;

use super::helpers::require_permission;
use crate::services::community_mek_local_rotate::rotate_mek_local;

/// Phase 8 — `idempotency_key` (UUID v7) dedupes click-spam. A double-
/// rotate without idempotency would desync MEK generations across the
/// community (mesh peers see two CryptoKeyDistributions and the second
/// supersedes the first, but local channel state may have already
/// flushed under the first key → silent decrypt failure on receivers).
#[tauri::command]
pub async fn rotate_mek(
    community_id: String,
    idempotency_key: uuid::Uuid,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    // Phase 5 — gate writes on lifecycle.
    let _g =
        rekindle_lifecycle::TransportGuard::write(&state.lifecycle).map_err(|e| e.to_string())?;
    let _ = pool;
    let state_clone = state.inner().clone();
    state
        .idempotency
        .wrap(idempotency_key, || async move {
            require_permission(&state_clone, &community_id, permissions::ADMINISTRATOR)?;
            rotate_mek_local(&app, &state_clone, &community_id).await?;
            tracing::info!(community = %community_id, "MEK rotated locally");
            Ok(())
        })
        .await
}
