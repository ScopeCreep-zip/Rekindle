//! Phase 23.D.9 — thin facade. All expression-asset Lost Cargo logic
//! (chunk + FEK encrypt + cache write + MEK wrap + plaintext read)
//! ported into `rekindle_files::{upload_expression_to_cache,
//! read_expression_bytes}` parameterised over `FilesDeps`.

use std::sync::Arc;

use rekindle_files::AttachmentOffer;
use tauri::Manager;

use crate::db::DbPool;
use crate::services::files_adapter::FilesAdapter;
use crate::state::{AppState, SharedState};

fn build_adapter(state: &SharedState) -> Result<Arc<FilesAdapter>, String> {
    let app_handle = state
        .app_handle
        .read()
        .clone()
        .ok_or_else(|| "app handle not initialized".to_string())?;
    let pool: tauri::State<'_, DbPool> = app_handle.state();
    Ok(FilesAdapter::new(
        state.clone(),
        app_handle.clone(),
        pool.inner().clone(),
    ))
}

pub fn upload_to_cache(
    state: &SharedState,
    community_id: &str,
    expression_id: [u8; 16],
    bytes: &[u8],
    filename: String,
    mime_type: String,
) -> Result<AttachmentOffer, String> {
    let adapter = build_adapter(state)?;
    rekindle_files::upload_expression_to_cache(
        adapter.as_ref(),
        community_id,
        expression_id,
        bytes,
        filename,
        mime_type,
    )
    .map_err(|e| e.to_string())
}

pub fn read_bytes_from_cache(
    state: &SharedState,
    community_id: &str,
    offer: &AttachmentOffer,
) -> Option<Vec<u8>> {
    let adapter = build_adapter(state).ok()?;
    rekindle_files::read_expression_bytes(adapter.as_ref(), community_id, offer)
}

/// Diff the merged `governance_state.expressions` against the local file
/// cache and broadcast a `RequestAttachment` for any missing chunks.
/// Architecture §18.4 line 2505 + §28.9 line 3286 — eager (automatic)
/// caching with no user action.
pub async fn eager_fetch_missing(state: &Arc<AppState>, community_id: &str) {
    let Some(app_handle) = state.app_handle.read().clone() else {
        tracing::debug!(community = %community_id, "eager expression fetch: no app handle yet");
        return;
    };
    let pool = {
        let Some(state_pool) = tauri::Manager::try_state::<crate::db::DbPool>(&app_handle) else {
            return;
        };
        state_pool.inner().clone()
    };
    let adapter = FilesAdapter::new(state.clone(), app_handle, pool);
    rekindle_files::eager_fetch_missing(adapter.as_ref(), community_id).await;
}
