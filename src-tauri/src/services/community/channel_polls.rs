//! Phase 23.D.5 — thin facade. All channel-polls business logic ported
//! into `rekindle_channel::polls` parameterised over `ChannelMessagingDeps`.
//! This module wraps the crate's orchestrators with `ChannelAdapter`
//! construction so existing command/runtime callers (which take
//! `&Arc<AppState>` + `community_id` + `channel_id` + payload) keep
//! their call shape unchanged.

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

pub async fn persist_poll_create(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
    message_id: &str,
    question: &str,
    answers: Vec<String>,
    multi_select: bool,
    duration_seconds: Option<u64>,
) -> Result<String, String> {
    let adapter = build_adapter(state)?;
    rekindle_channel::persist_poll_create(
        &adapter,
        community_id,
        channel_id,
        message_id,
        question,
        answers,
        multi_select,
        duration_seconds,
    )
    .await
    .map_err(|e| e.to_string())
}

pub async fn persist_poll_vote(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
    poll_id_hex: &str,
    selected_answers: Vec<u8>,
) -> Result<(), String> {
    let adapter = build_adapter(state)?;
    rekindle_channel::persist_poll_vote(
        &adapter,
        community_id,
        channel_id,
        poll_id_hex,
        selected_answers,
    )
    .await
    .map_err(|e| e.to_string())
}

pub async fn persist_poll_close(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
    poll_id_hex: &str,
    allow_moderator_override: bool,
) -> Result<(), String> {
    let adapter = build_adapter(state)?;
    rekindle_channel::persist_poll_close(
        &adapter,
        community_id,
        channel_id,
        poll_id_hex,
        allow_moderator_override,
    )
    .await
    .map_err(|e| e.to_string())
}

pub async fn get_poll_results(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
    poll_id_hex: &str,
) -> Result<Vec<u32>, String> {
    let adapter = build_adapter(state)?;
    rekindle_channel::get_poll_results(&adapter, community_id, channel_id, poll_id_hex)
        .await
        .map_err(|e| e.to_string())
}
