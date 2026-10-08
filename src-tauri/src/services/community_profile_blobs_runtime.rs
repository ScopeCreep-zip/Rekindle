//! Phase 23.C — community-profile blob helpers lifted from
//! `commands/community/profile_blobs.rs`. Hosts the WebP compression
//! pipeline + `store` orchestrator (validate raw bounds, compress on a
//! blocking thread, content-address, write to the per-community cache
//! dir, return BLAKE3 hash) and the read-side `get_data_url`.

use std::io::Cursor;
use std::path::PathBuf;

use image::ImageReader;

use crate::state::SharedState;

pub const AVATAR_MAX_DIM: u32 = 128;
pub const BANNER_MAX_W: u32 = 600;
pub const BANNER_MAX_H: u32 = 200;
pub const MAX_RAW_AVATAR_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_RAW_BANNER_BYTES: usize = 8 * 1024 * 1024;

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

pub async fn store_blob(
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
        return Err(format!("payload exceeds {} KiB raw budget", max_raw / 1024));
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

pub fn read_data_url(
    state: &SharedState,
    community_id: &str,
    hash: &str,
) -> Result<Option<String>, String> {
    use base64::Engine as _;
    let dir = cache_dir(state, community_id)?;
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
