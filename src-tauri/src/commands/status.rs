use std::io::Cursor;

use image::ImageReader;
use tauri::{Emitter as _, State};

use crate::commands::auth::current_owner_key;
use crate::db::DbPool;
use crate::services;
use crate::state::{SharedState, UserStatus};

/// Set online status and publish to DHT.
#[tauri::command]
pub async fn set_status(
    status: String,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    let status_enum = match status.as_str() {
        "online" => UserStatus::Online,
        "away" => UserStatus::Away,
        "busy" => UserStatus::Busy,
        "offline" => UserStatus::Offline,
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

    // Publish to DHT profile subkey 2
    services::presence_service::publish_status(state.inner(), status_enum).await?;

    Ok(())
}

/// Set display name, persist to `SQLite`, and push update to DHT.
#[tauri::command]
pub async fn set_nickname(
    nickname: String,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let public_key = {
        let mut identity = state.identity.write();
        let id = identity.as_mut().ok_or("not logged in")?;
        id.display_name = nickname.clone();
        id.public_key.clone()
    };

    // Persist to SQLite
    let pool = pool.inner().clone();
    let nickname_clone = nickname.clone();
    let pk_clone = public_key.clone();
    tokio::task::spawn_blocking(move || {
        let conn = pool.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE identity SET display_name = ? WHERE public_key = ?",
            rusqlite::params![nickname_clone, pk_clone],
        )
        .map_err(|e| e.to_string())?;
        Ok::<_, String>(())
    })
    .await
    .map_err(|e| e.to_string())??;

    // Notify all windows so they refresh their local auth store
    let _ = app.emit("profile-updated", ());

    // Push display name to DHT profile subkey 0
    services::message_service::push_profile_update(
        state.inner(),
        0,
        nickname.into_bytes(),
    )
    .await
}

/// Maximum avatar dimension (width or height) in pixels.
const AVATAR_MAX_DIM: u32 = 128;

/// Compress raw image bytes to a 128x128 WebP avatar.
///
/// Runs on a blocking thread because image decoding/encoding is CPU-bound.
fn compress_avatar_to_webp(raw: &[u8]) -> Result<Vec<u8>, String> {
    let img = ImageReader::new(Cursor::new(&raw))
        .with_guessed_format()
        .map_err(|e| format!("failed to guess image format: {e}"))?
        .decode()
        .map_err(|e| format!("failed to decode image: {e}"))?;

    // Resize only if larger than max dimension, preserving aspect ratio
    let resized = if img.width() > AVATAR_MAX_DIM || img.height() > AVATAR_MAX_DIM {
        img.resize(
            AVATAR_MAX_DIM,
            AVATAR_MAX_DIM,
            image::imageops::FilterType::Lanczos3,
        )
    } else {
        img
    };

    // Encode to WebP
    let mut webp_buf: Vec<u8> = Vec::new();
    resized
        .write_to(
            &mut Cursor::new(&mut webp_buf),
            image::ImageFormat::WebP,
        )
        .map_err(|e| format!("failed to encode WebP: {e}"))?;

    Ok(webp_buf)
}

/// Set avatar image: compress to WebP, persist to `SQLite`, and push update to DHT.
#[tauri::command]
pub async fn set_avatar(
    avatar_data: Vec<u8>,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    // Compress on a blocking thread (CPU-bound work)
    let webp_bytes = tokio::task::spawn_blocking(move || compress_avatar_to_webp(&avatar_data))
        .await
        .map_err(|e| e.to_string())??;

    // Get our public key from identity (clone out before .await)
    let public_key = {
        let identity = state.identity.read();
        identity
            .as_ref()
            .ok_or("not logged in")?
            .public_key
            .clone()
    };

    // Persist to SQLite
    let pool_clone = pool.inner().clone();
    let pk_clone = public_key.clone();
    let webp_for_db = webp_bytes.clone();
    tokio::task::spawn_blocking(move || {
        let conn = pool_clone.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE identity SET avatar_webp = ? WHERE public_key = ?",
            rusqlite::params![webp_for_db, pk_clone],
        )
        .map_err(|e| e.to_string())?;
        Ok::<_, String>(())
    })
    .await
    .map_err(|e| e.to_string())??;

    tracing::info!(
        public_key = %public_key,
        webp_size = webp_bytes.len(),
        "avatar compressed and persisted"
    );

    // Notify all windows so they refresh their local auth store
    let _ = app.emit("profile-updated", ());

    // Push avatar to DHT profile subkey 3
    services::message_service::push_profile_update(state.inner(), 3, webp_bytes).await
}

/// Retrieve a user's avatar as WebP bytes.
///
/// Returns the avatar for the given `public_key`. Checks the `identity` table
/// first (for our own avatar), then falls back to the `friends` table
/// scoped to the current identity's `owner_key`.
#[tauri::command]
pub async fn get_avatar(
    public_key: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<Option<Vec<u8>>, String> {
    let owner_key = current_owner_key(state.inner()).unwrap_or_default();
    let pool = pool.inner().clone();
    tokio::task::spawn_blocking(move || {
        let conn = pool.lock().map_err(|e| e.to_string())?;

        // Try identity table first (our own avatar)
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

        // Fall back to friends table (scoped to current identity)
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
    .map_err(|e| e.to_string())?
}

/// Set status message and push update to DHT.
#[tauri::command]
pub async fn set_status_message(
    message: String,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    if let Some(ref mut identity) = *state.identity.write() {
        identity.status_message.clone_from(&message);
    }
    // Push status message to DHT profile subkey 1
    services::message_service::push_profile_update(
        state.inner(),
        1,
        message.into_bytes(),
    )
    .await
}
