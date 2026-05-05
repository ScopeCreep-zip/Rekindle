//! Architecture §32 Phase 5 Week 15 — per-community avatar + banner.
//!
//! Profile images are content-addressed under
//! `<app_data>/community_avatars/<community_id>/<blake3_hex>.webp`.
//! Uploads compress to WebP (avatars 128×128, banners 600×200), hash
//! the compressed bytes with BLAKE3, store under the hex hash, and
//! return the hash for inclusion in `MemberPresence.avatar_ref` /
//! `banner_ref`. Reads return a `data:image/webp;base64,...` URL the
//! frontend can drop straight into an `<img src=…>`.
//!
//! Peer-to-peer distribution of these blobs (so members can see each
//! other's avatars) rides the existing Lost Cargo attachment fetch
//! path; this command pair handles the local cache only.

use std::io::Cursor;
use std::path::PathBuf;

use image::ImageReader;
use tauri::State;

use crate::state::SharedState;

const AVATAR_MAX_DIM: u32 = 128;
const BANNER_MAX_W: u32 = 600;
const BANNER_MAX_H: u32 = 200;
const MAX_RAW_AVATAR_BYTES: usize = 4 * 1024 * 1024;
const MAX_RAW_BANNER_BYTES: usize = 8 * 1024 * 1024;

fn cache_dir(state: &SharedState, community_id: &str) -> Result<PathBuf, String> {
    let root = state
        .file_cache_root
        .read()
        .clone()
        .ok_or_else(|| "file cache root not initialized".to_string())?;
    let dir = root.join("community_avatars").join(community_id);
    std::fs::create_dir_all(&dir).map_err(|e| format!("create cache dir: {e}"))?;
    Ok(dir)
}

fn compress_to_webp(raw: &[u8], max_w: u32, max_h: u32) -> Result<Vec<u8>, String> {
    let img = ImageReader::new(Cursor::new(raw))
        .with_guessed_format()
        .map_err(|e| format!("guess format: {e}"))?
        .decode()
        .map_err(|e| format!("decode: {e}"))?;
    let resized = if img.width() > max_w || img.height() > max_h {
        img.resize(max_w, max_h, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };
    let mut buf: Vec<u8> = Vec::new();
    resized
        .write_to(&mut Cursor::new(&mut buf), image::ImageFormat::WebP)
        .map_err(|e| format!("encode webp: {e}"))?;
    Ok(buf)
}

async fn store(
    state: &SharedState,
    community_id: &str,
    raw: Vec<u8>,
    max_raw: usize,
    max_w: u32,
    max_h: u32,
) -> Result<String, String> {
    if raw.is_empty() {
        return Err("payload empty".into());
    }
    if raw.len() > max_raw {
        return Err(format!(
            "payload exceeds {} KiB raw budget",
            max_raw / 1024
        ));
    }
    let webp = tokio::task::spawn_blocking(move || compress_to_webp(&raw, max_w, max_h))
        .await
        .map_err(|e| e.to_string())??;
    let hash = blake3::hash(&webp).to_hex().to_string();
    let dir = cache_dir(state, community_id)?;
    let path = dir.join(format!("{hash}.webp"));
    if !path.exists() {
        std::fs::write(&path, &webp).map_err(|e| format!("write blob: {e}"))?;
    }
    Ok(hash)
}

#[tauri::command]
pub async fn set_community_avatar(
    state: State<'_, SharedState>,
    community_id: String,
    bytes: Vec<u8>,
) -> Result<String, String> {
    store(
        state.inner(),
        &community_id,
        bytes,
        MAX_RAW_AVATAR_BYTES,
        AVATAR_MAX_DIM,
        AVATAR_MAX_DIM,
    )
    .await
}

#[tauri::command]
pub async fn set_community_banner(
    state: State<'_, SharedState>,
    community_id: String,
    bytes: Vec<u8>,
) -> Result<String, String> {
    store(
        state.inner(),
        &community_id,
        bytes,
        MAX_RAW_BANNER_BYTES,
        BANNER_MAX_W,
        BANNER_MAX_H,
    )
    .await
}

#[tauri::command]
pub async fn get_community_avatar_data_url(
    state: State<'_, SharedState>,
    community_id: String,
    hash: String,
) -> Result<Option<String>, String> {
    use base64::Engine as _;
    let dir = cache_dir(state.inner(), &community_id)?;
    let path = dir.join(format!("{hash}.webp"));
    match std::fs::read(&path) {
        Ok(bytes) => {
            let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
            Ok(Some(format!("data:image/webp;base64,{b64}")))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("read blob: {e}")),
    }
}
