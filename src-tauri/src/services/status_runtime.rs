//! Phase 23.C — status-handler Tauri-runtime orchestration lifted from
//! `commands/status.rs`. Hosts:
//! * `set_status_inner` — validate + state mutation + DHT publish.
//! * `set_nickname_inner` — DB write + DHT subkey 0 push.
//! * `compress_avatar_to_webp` + `set_avatar_inner` — image compress to
//!   WebP (CPU-bound, on a blocking thread) + DB write + DHT subkey 3
//!   push.
//! * `get_avatar_inner` — two-step SQLite lookup (identity → friends).
//! * `set_status_message_inner` — state mutation + DHT subkey 1 push.

use std::io::Cursor;

use image::ImageReader;

use crate::db::DbPool;
use crate::db_helpers::db_call;
use crate::services;
use crate::state::{SharedState, UserStatus};
use crate::state_helpers;

pub async fn set_status_inner(state: &SharedState, status: String) -> Result<(), String> {
    let status_enum = match status.as_str() {
        "online" => UserStatus::Online,
        "away" => UserStatus::Away,
        "busy" => UserStatus::Busy,
        "offline" => UserStatus::Offline,
        "invisible" => UserStatus::Invisible,
        _ => return Err(format!("invalid status: {status}")),
    };

    if let Some(ref mut identity) = *state.identity.write() {
        identity.status = status_enum;
        tracing::info!(
            status = ?status_enum,
            status_message = %identity.status_message,
            "status updated"
        );
    }

    *state.pre_away_status.write() = None;

    services::presence_service::publish_status(state, status_enum).await?;

    Ok(())
}

pub async fn set_nickname_inner(
    state: &SharedState,
    pool: &DbPool,
    app: &tauri::AppHandle,
    nickname: String,
) -> Result<(), String> {
    let public_key = {
        let mut identity = state.identity.write();
        let id = identity.as_mut().ok_or("not logged in")?;
        id.display_name = nickname.clone();
        id.public_key.clone()
    };

    let nickname_clone = nickname.clone();
    let pk_clone = public_key.clone();
    db_call(pool, move |conn| {
        conn.execute(
            "UPDATE identity SET display_name = ? WHERE public_key = ?",
            rusqlite::params![nickname_clone, pk_clone],
        )?;
        Ok(())
    })
    .await?;

    crate::event_dispatch::emit_live(app, "profile-updated", &());

    services::message_service::push_profile_update(state, 0, nickname.into_bytes()).await
}

const AVATAR_MAX_DIM: u32 = 128;

fn compress_avatar_to_webp(raw: &[u8]) -> Result<Vec<u8>, String> {
    let img = ImageReader::new(Cursor::new(&raw))
        .with_guessed_format()
        .map_err(|e| format!("failed to guess image format: {e}"))?
        .decode()
        .map_err(|e| format!("failed to decode image: {e}"))?;

    let resized = if img.width() > AVATAR_MAX_DIM || img.height() > AVATAR_MAX_DIM {
        img.resize(
            AVATAR_MAX_DIM,
            AVATAR_MAX_DIM,
            image::imageops::FilterType::Lanczos3,
        )
    } else {
        img
    };

    let mut webp_buf: Vec<u8> = Vec::new();
    resized
        .write_to(&mut Cursor::new(&mut webp_buf), image::ImageFormat::WebP)
        .map_err(|e| format!("failed to encode WebP: {e}"))?;

    Ok(webp_buf)
}

pub async fn set_avatar_inner(
    state: &SharedState,
    pool: &DbPool,
    app: &tauri::AppHandle,
    avatar_data: Vec<u8>,
) -> Result<(), String> {
    let webp_bytes = tokio::task::spawn_blocking(move || compress_avatar_to_webp(&avatar_data))
        .await
        .map_err(|e| e.to_string())??;

    let public_key = state_helpers::current_owner_key(state)?;

    let pk_clone = public_key.clone();
    let webp_for_db = webp_bytes.clone();
    db_call(pool, move |conn| {
        conn.execute(
            "UPDATE identity SET avatar_webp = ? WHERE public_key = ?",
            rusqlite::params![webp_for_db, pk_clone],
        )?;
        Ok(())
    })
    .await?;

    tracing::info!(
        public_key = %public_key,
        webp_size = webp_bytes.len(),
        "avatar compressed and persisted"
    );

    crate::event_dispatch::emit_live(app, "profile-updated", &());

    services::message_service::push_profile_update(state, 3, webp_bytes).await
}

pub async fn get_avatar_inner(
    state: &SharedState,
    pool: &DbPool,
    public_key: String,
) -> Result<Option<Vec<u8>>, String> {
    let owner_key = state_helpers::owner_key_or_default(state);
    db_call(pool, move |conn| {
        let own: Option<Vec<u8>> = conn
            .query_row(
                "SELECT avatar_webp FROM identity WHERE public_key = ?1",
                rusqlite::params![public_key],
                |row| row.get(0),
            )
            .ok()
            .flatten();

        if own.is_some() {
            return Ok(own);
        }

        let friend: Option<Vec<u8>> = conn
            .query_row(
                "SELECT avatar_webp FROM friends WHERE owner_key = ?1 AND public_key = ?2",
                rusqlite::params![owner_key, public_key],
                |row| row.get(0),
            )
            .ok()
            .flatten();

        Ok(friend)
    })
    .await
}

pub async fn set_status_message_inner(
    state: &SharedState,
    message: String,
) -> Result<(), String> {
    if let Some(ref mut identity) = *state.identity.write() {
        identity.status_message.clone_from(&message);
    }
    services::message_service::push_profile_update(state, 1, message.into_bytes()).await
}
