use tauri::State;

use crate::db::DbPool;
use crate::services::community_channel_admin_runtime::{
    delete_channel_inner, rename_channel_inner,
};
use crate::state::SharedState;

/// Phase 8 — `idempotency_key` (UUID v7) dedupes click-spam. Without
/// it, two rapid clicks each write a `ChannelArchived` lamport entry;
/// the second is a no-op locally but still consumes a lamport tick
/// (governance-state divergence vs. peers).
#[tauri::command]
pub async fn delete_channel(
    community_id: String,
    channel_id: String,
    idempotency_key: uuid::Uuid,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let _g =
        rekindle_lifecycle::TransportGuard::write(&state.lifecycle).map_err(|e| e.to_string())?;
    let s = state.inner().clone();
    let p = pool.inner().clone();
    state
        .idempotency
        .wrap(idempotency_key, || async move {
            delete_channel_inner(&s, &p, community_id, channel_id).await
        })
        .await
}

#[tauri::command]
pub async fn rename_channel(
    community_id: String,
    channel_id: String,
    new_name: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    rename_channel_inner(
        state.inner(),
        pool.inner(),
        community_id,
        channel_id,
        new_name,
    )
    .await
}
