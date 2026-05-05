//! Tauri command for sender-side link preview generation
//! (architecture §28.8). The frontend detects URLs in composition,
//! calls this once per URL, and the backend handles fetching and
//! gossip broadcast.

use rekindle_types::link_preview::LinkPreview;
use tauri::State;

use crate::db::DbPool;
use crate::db_helpers::{db_call, db_call_or_default};
use crate::services::community::link_previews;
use crate::state::SharedState;
use crate::state_helpers;

#[tauri::command]
pub async fn fetch_link_preview(
    community_id: String,
    channel_id: String,
    message_id: String,
    url: String,
    state: State<'_, SharedState>,
) -> Result<LinkPreview, String> {
    link_previews::fetch_and_broadcast(
        state.inner(),
        &community_id,
        &channel_id,
        &message_id,
        &url,
    )
    .await
}

/// Architecture §28.8 line 3220 — IP-privacy toggle for link preview
/// generation. When `false`, the OpenGraph fetch is skipped; URLs in
/// outgoing messages remain bare (receivers still see the URL but no
/// inline card). Affects only this device — receivers' previews from
/// other senders continue to render.
#[tauri::command]
pub async fn set_link_previews_enabled(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    enabled: bool,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    let value = i64::from(enabled);
    db_call(pool.inner(), move |conn| {
        conn.execute(
            "INSERT INTO app_settings (owner_key, link_previews_enabled) VALUES (?1, ?2) \
             ON CONFLICT(owner_key) DO UPDATE SET link_previews_enabled = excluded.link_previews_enabled",
            rusqlite::params![owner_key, value],
        )?;
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn get_link_previews_enabled(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<bool, String> {
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    Ok(db_call_or_default(pool.inner(), move |conn| {
        let value: Option<i64> = conn
            .query_row(
                "SELECT link_previews_enabled FROM app_settings WHERE owner_key = ?1",
                rusqlite::params![owner_key],
                |row| row.get(0),
            )
            .ok();
        Ok(value.unwrap_or(1) != 0)
    })
    .await)
}
