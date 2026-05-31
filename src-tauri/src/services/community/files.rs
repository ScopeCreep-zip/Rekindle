//! Lost Cargo (file sharing) service module.
//!
//! Architecture §28.9 — chunked attachment delivery over Veilid:
//!  - chunks live in a local per-community filesystem cache (`rekindle-files`)
//!  - the announcement (`AttachmentOffer`) travels embedded in a
//!    `ChannelEntry::Message` (architecture line 3233)
//!  - peers write `AttachmentCached` entries to their SMPL subkeys to
//!    advertise possession; downloaders scan those entries to find sources
//!  - chunks themselves move via `app_call` (`ControlPayload::AttachmentChunk`)
//!
//! This module is the wiring layer between `rekindle-files` (pure logic),
//! `rekindle-protocol` (wire types + DHT helpers), and the Tauri command
//! handlers in `commands/community/files.rs`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rekindle_files::{CacheConfig, ChunkCache};
use tauri::AppHandle;

use crate::db::DbPool;
use crate::state::{AppState, SharedState};

/// 1 GB per spec §28.9 line 3283.
const DEFAULT_BYTE_BUDGET: u64 = 1024 * 1024 * 1024;

// ─── Phase 3: cache lifecycle ──────────────────────────────────────────

/// Resolve the per-community cache directory under the global file_cache root.
fn community_cache_dir(state: &SharedState, community_id: &str) -> Option<PathBuf> {
    let root = state.file_cache_root.read().clone()?;
    Some(root.join(community_id))
}

/// Ensure the chunk cache for a given community is open. Idempotent.
pub fn ensure_cache_open(state: &SharedState, community_id: &str) -> Result<(), String> {
    if state.file_caches.read().contains_key(community_id) {
        return Ok(());
    }
    let dir = community_cache_dir(state, community_id)
        .ok_or_else(|| "file cache root not initialized".to_string())?;
    let cache = ChunkCache::open(CacheConfig {
        root_dir: dir,
        byte_budget: DEFAULT_BYTE_BUDGET,
    })
    .map_err(|e| format!("failed to open file cache: {e}"))?;
    state
        .file_caches
        .write()
        .entry(community_id.to_string())
        .or_insert(cache);
    state
        .pinned_attachments
        .write()
        .entry(community_id.to_string())
        .or_default();
    Ok(())
}

/// Sync the in-memory pinned set for a community from the merged governance
/// state's `pinned_attachments`. Run after every governance merge.
/// Phase 23.D.8 — orchestration ported into `rekindle_files::sync_pinned_from_governance`.
pub fn sync_pinned_from_governance(state: &SharedState, community_id: &str) {
    let Ok(adapter) = pinned_adapter(state) else {
        return;
    };
    rekindle_files::sync_pinned_from_governance(adapter.as_ref(), community_id);
}

fn pinned_adapter(
    state: &SharedState,
) -> Result<std::sync::Arc<crate::services::files_adapter::FilesAdapter>, String> {
    let app_handle = state
        .app_handle
        .read()
        .clone()
        .ok_or_else(|| "app handle not initialized".to_string())?;
    let pool: tauri::State<'_, DbPool> = tauri::Manager::state::<DbPool>(&app_handle);
    Ok(crate::services::files_adapter::FilesAdapter::new(
        state.clone(),
        app_handle.clone(),
        pool.inner().clone(),
    ))
}

// ─── Phase 4: upload — Phase 15 moved into rekindle_files::upload ─────
//
// `AttachmentRecordJson`, `UploadContext`, `build_upload_context`,
// `write_self_attachment_cached`, `next_channel_sequence`,
// `insert_message_full_attachment`, `guess_mime_type` all live in the
// crate now. The three public entry points (upload_file,
// upload_bytes_as_attachment, send_voice_message_bytes) keep their
// pre-Phase-15 signatures here as thin facades that construct a
// FilesAdapter + delegate to the crate.

/// Helper: build a FilesAdapter from an upload callsite that has
/// `&SharedState + &DbPool` (no AppHandle on the call surface). We
/// retrieve the AppHandle from state.app_handle, falling back to an
/// error if it isn't yet wired (commands run after setup so this
/// should never trigger in practice).
fn build_files_adapter(
    state: &SharedState,
    pool: &DbPool,
) -> Result<std::sync::Arc<crate::services::files_adapter::FilesAdapter>, String> {
    let app_handle = state
        .app_handle
        .read()
        .clone()
        .ok_or_else(|| "app handle not initialized".to_string())?;
    Ok(crate::services::files_adapter::FilesAdapter::new(
        state.clone(),
        app_handle,
        pool.clone(),
    ))
}

// ─────────────────────────────────────────────────────────────────────
// Thin facades around `rekindle_files::upload`. The pre-Phase-15
// signatures are preserved so commands/community/files.rs callsites
// don't need updates. All upload orchestration logic now lives in the
// crate.
// ─────────────────────────────────────────────────────────────────────

/// Read a file from disk + delegate to [`upload_bytes_as_attachment`].
pub async fn upload_file(
    state: &SharedState,
    pool: &crate::db::DbPool,
    community_id: &str,
    channel_id: &str,
    file_path: &Path,
) -> Result<String, String> {
    let adapter = build_files_adapter(state, pool)?;
    rekindle_files::upload_file(adapter.as_ref(), community_id, channel_id, file_path)
        .await
        .map_err(|e| e.to_string())
}

// `upload_bytes_as_attachment` facade deleted — no src-tauri caller
// exists; in-crate callers (upload_file, send_voice_message_bytes)
// use `rekindle_files::upload_bytes_as_attachment` directly. Future
// callers should import from the crate.

/// Architecture §16.4 — voice message upload facade. Delegates to
/// [`rekindle_files::send_voice_message_bytes`].
pub async fn send_voice_message_bytes(
    state: &SharedState,
    pool: &crate::db::DbPool,
    community_id: &str,
    channel_id: &str,
    opus_bytes: Vec<u8>,
    duration_ms: u32,
    waveform: Vec<u8>,
) -> Result<String, String> {
    let adapter = build_files_adapter(state, pool)?;
    rekindle_files::send_voice_message_bytes(
        adapter.as_ref(),
        community_id,
        channel_id,
        opus_bytes,
        duration_ms,
        waveform,
    )
    .await
    .map_err(|e| e.to_string())
}

// `UploadContext`, `build_upload_context`, `write_self_attachment_cached`,
// `next_channel_sequence`, `insert_message_full_attachment`, and
// `guess_mime_type` deleted (relocated to `rekindle_files::upload`).
// The crate's `FilesDeps` trait + the adapter's
// `write_channel_message_to_smpl`/`write_attachment_cached_to_smpl`/
// `insert_channel_message_full`/`persist_slowmode_state`/
// `next_channel_sequence` methods supply the equivalent behavior.

// [Phase 15 marker — block from `UploadContext` through `guess_mime_type`
// (UploadContext + build_upload_context + upload_file + upload_bytes_as_attachment
// + write_self_attachment_cached + next_channel_sequence +
// insert_message_full_attachment + send_voice_message_bytes + guess_mime_type)
// is deleted below this anchor; logic is in `rekindle_files::upload`.
// The thin facades earlier in this file delegate via FilesAdapter.]


// ─── Phase 4: serve chunks (responder side of app_call) ────────────────

/// Handle an incoming `RequestAttachment` control payload — a peer wants
/// chunks of an attachment we may have cached. We reply with a
/// `MultiAttachmentChunk` envelope containing each chunk we hold from the
/// requested set. Returns the serialized reply bytes (an
/// `app_call_reply` payload) — `None` if we have nothing to offer.
///
/// Phase 15 — body extracted to `rekindle_files::serve::serve_attachment_request`
/// (pure, takes `&mut ChunkCache`). This facade owns the AppState lock
/// + per-community cache lookup.
pub fn serve_attachment_request(
    state: &Arc<AppState>,
    community_id: &str,
    attachment_id: [u8; 16],
    requested_chunks: &[u32],
) -> Option<Vec<u8>> {
    let mut caches = state.file_caches.write();
    let cache = caches.get_mut(community_id)?;
    rekindle_files::serve_attachment_request(cache, attachment_id, requested_chunks)
}

// ─── Phase 4: download (consumer side of app_call) ─────────────────────

/// Phase 15 — `download_attachment` body moved into
/// `rekindle_files::download`. This facade preserves the prior
/// signature so command callsites don't need updates; it builds a
/// FilesAdapter + delegates.
pub async fn download_attachment(
    state: &SharedState,
    pool: &crate::db::DbPool,
    community_id: &str,
    channel_id: &str,
    attachment_id_hex: &str,
    save_path: &Path,
) -> Result<(), String> {
    let adapter = build_files_adapter(state, pool)?;
    rekindle_files::download_attachment(
        adapter.as_ref(),
        community_id,
        channel_id,
        attachment_id_hex,
        save_path,
    )
    .await
    .map_err(|e| e.to_string())
}

// ─── Phase 4: pin / unpin command body ─────────────────────────────────

/// Phase 23.D.8 — orchestration ported into `rekindle_files::set_attachment_pinned`.
pub async fn set_attachment_pinned(
    state: &SharedState,
    community_id: &str,
    attachment_id_hex: &str,
    pinned: bool,
) -> Result<(), String> {
    let adapter = pinned_adapter(state)?;
    rekindle_files::set_attachment_pinned(adapter.as_ref(), community_id, attachment_id_hex, pinned)
        .await
        .map_err(|e| e.to_string())
}

// ─── Phase 4: progress event for the UI ────────────────────────────────

pub fn emit_attachment_complete(
    app_handle: &AppHandle,
    community_id: &str,
    channel_id: &str,
    attachment_id_hex: &str,
    local_path: &Path,
) {
    crate::event_dispatch::emit_live(
        app_handle,
        "community-event",
        &crate::channels::CommunityEvent::AttachmentDownloaded {
            community_id: community_id.to_string(),
            channel_id: channel_id.to_string(),
            attachment_id: attachment_id_hex.to_string(),
            local_path: local_path.display().to_string(),
        },
    );
}
