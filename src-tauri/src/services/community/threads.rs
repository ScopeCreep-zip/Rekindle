//! Phase 19.i-REDO — thin facade.
//!
//! Thread orchestration (create / list / send / load / archive) lives
//! in `rekindle_channel::threads`. This module constructs a
//! `ChannelAdapter` per call and maps the crate's
//! `ThreadInfoSnapshot` ↔ src-tauri `ThreadInfoDto` and
//! `ThreadMessageView` ↔ src-tauri `Message`.

use std::sync::Arc;

use tauri::Manager;

use crate::channels::community_channel::ThreadInfoDto;
use crate::commands::chat::Message;
use crate::db::DbPool;
use crate::state::SharedState;

fn build_adapter(
    state: &SharedState,
) -> Result<crate::services::channel_adapter::ChannelAdapter, String> {
    let app_handle = state
        .app_handle
        .read()
        .clone()
        .ok_or_else(|| "app handle unavailable".to_string())?;
    let pool = app_handle
        .try_state::<DbPool>()
        .ok_or_else(|| "DbPool state missing".to_string())?
        .inner()
        .clone();
    Ok(crate::services::channel_adapter::ChannelAdapter::new(
        Arc::clone(state),
        app_handle,
        pool,
    ))
}

fn snapshot_to_dto(snapshot: rekindle_channel::deps::ThreadInfoSnapshot) -> ThreadInfoDto {
    ThreadInfoDto {
        id: snapshot.id,
        channel_id: snapshot.channel_id,
        name: snapshot.name,
        starter_message_id: snapshot.starter_message_id,
        creator_pseudonym: snapshot.creator_pseudonym,
        forum_tag: snapshot.forum_tag,
        created_at: snapshot.created_at,
        archived: snapshot.archived,
        auto_archive_seconds: snapshot.auto_archive_seconds,
        last_message_at: snapshot.last_message_at,
        message_count: snapshot.message_count,
    }
}

fn view_to_message(view: rekindle_channel::ThreadMessageView) -> Message {
    Message {
        id: 0,
        sender_id: view.sender_pseudonym,
        body: view.body,
        decryption_failed: false,
        automod_blurred: false,
        timestamp: i64::try_from(view.timestamp_ms).unwrap_or(i64::MAX),
        is_own: view.is_own,
        server_message_id: view.server_message_id,
        reactions: None,
        pinned: None,
        poll: None,
        forwarded_from_author: None,
        attachment: None,
        flags: 0,
    }
}

pub async fn create_thread(
    state: &SharedState,
    _pool: &DbPool,
    community_id: &str,
    channel_id: &str,
    name: &str,
    starter_message_id: &str,
    forum_tag: Option<String>,
    auto_archive_override: Option<u64>,
) -> Result<String, String> {
    let adapter = build_adapter(state)?;
    rekindle_channel::create_thread(
        &adapter,
        community_id,
        channel_id,
        name,
        starter_message_id,
        forum_tag,
        auto_archive_override,
    )
    .await
    .map_err(|e| e.to_string())
}

pub async fn list_threads(
    state: &SharedState,
    _pool: &DbPool,
    community_id: &str,
    channel_id: &str,
) -> Result<Vec<ThreadInfoDto>, String> {
    let adapter = build_adapter(state)?;
    let snapshots = rekindle_channel::list_threads(&adapter, community_id, channel_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(snapshots.into_iter().map(snapshot_to_dto).collect())
}

pub async fn list_active_threads(
    state: &SharedState,
    _pool: &DbPool,
    community_id: &str,
    channel_id: &str,
) -> Result<Vec<ThreadInfoDto>, String> {
    let adapter = build_adapter(state)?;
    let snapshots = rekindle_channel::list_active_threads(&adapter, community_id, channel_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(snapshots.into_iter().map(snapshot_to_dto).collect())
}

pub async fn send_thread_message(
    state: &SharedState,
    community_id: &str,
    thread_id: &str,
    body: &str,
) -> Result<(), String> {
    let adapter = build_adapter(state)?;
    rekindle_channel::send_thread_message(&adapter, community_id, thread_id, body)
        .await
        .map_err(|e| e.to_string())
}

pub async fn load_thread_messages(
    state: &SharedState,
    community_id: &str,
    thread_id: &str,
    limit: u32,
    before_timestamp: Option<u64>,
) -> Result<Vec<Message>, String> {
    let adapter = build_adapter(state)?;
    let views = rekindle_channel::load_thread_messages(
        &adapter,
        community_id,
        thread_id,
        limit,
        before_timestamp,
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(views.into_iter().map(view_to_message).collect())
}

pub async fn archive_thread(
    state: &SharedState,
    community_id: &str,
    thread_id: &str,
) -> Result<(), String> {
    let adapter = build_adapter(state)?;
    rekindle_channel::archive_thread(&adapter, community_id, thread_id)
        .await
        .map_err(|e| e.to_string())
}
