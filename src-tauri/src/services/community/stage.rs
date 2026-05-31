//! Phase 23.D.7 — thin facade. All voice-stage hand-raise SMPL
//! protocol logic lives in `rekindle_channel::stage` parameterised
//! over `ChannelMessagingDeps`.

use std::sync::Arc;

use tauri::Manager;

use crate::db::DbPool;
use crate::services::channel_adapter::ChannelAdapter;
use crate::state::AppState;

fn build_adapter(state: &Arc<AppState>) -> Result<ChannelAdapter, String> {
    let app_handle = state
        .app_handle
        .read()
        .clone()
        .ok_or("app handle not initialized")?;
    let pool: tauri::State<'_, DbPool> = app_handle.state();
    Ok(ChannelAdapter::new(
        Arc::clone(state),
        app_handle.clone(),
        pool.inner().clone(),
    ))
}

pub async fn persist_hand_raise(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
    raised: bool,
) -> Result<(), String> {
    let adapter = build_adapter(state)?;
    rekindle_channel::persist_hand_raise(&adapter, community_id, channel_id, raised)
        .await
        .map_err(|e| e.to_string())
}

pub async fn list_hand_raises(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
) -> Result<Vec<String>, String> {
    let adapter = build_adapter(state)?;
    rekindle_channel::list_hand_raises(&adapter, community_id, channel_id)
        .await
        .map_err(|e| e.to_string())
}
