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

use crate::state::SharedState;
use rekindle_db::Db;
use std::path::Path;

// ─── Phase 3: cache lifecycle ──────────────────────────────────────────

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
    let pool = state.db.current()?;
    Ok(crate::services::files_adapter::FilesAdapter::new(
        state.clone(),
        app_handle.clone(),
        pool,
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
/// `&SharedState + &Db` (no AppHandle on the call surface). We
/// retrieve the AppHandle from state.app_handle, falling back to an
/// error if it isn't yet wired (commands run after setup so this
/// should never trigger in practice).
fn build_files_adapter(
    state: &SharedState,
    pool: &Db,
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
    pool: &rekindle_db::Db,
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
    pool: &rekindle_db::Db,
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

// ─── Phase 4: download (consumer side of app_call) ─────────────────────

/// Phase 15 — `download_attachment` body moved into
/// `rekindle_files::download`. This facade preserves the prior
/// signature so command callsites don't need updates; it builds a
/// FilesAdapter + delegates.
pub async fn download_attachment(
    state: &SharedState,
    pool: &rekindle_db::Db,
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

/// The plaintext of an attachment, fetched into the chunk cache if needed.
/// Voice messages play from these bytes; nothing is written outside the
/// cache.
pub async fn attachment_bytes(
    state: &SharedState,
    pool: &rekindle_db::Db,
    community_id: &str,
    channel_id: &str,
    attachment_id_hex: &str,
) -> Result<Vec<u8>, String> {
    let adapter = build_files_adapter(state, pool)?;
    let fetched = rekindle_files::fetch_attachment_to_cache(
        adapter.as_ref(),
        community_id,
        channel_id,
        attachment_id_hex,
    )
    .await
    .map_err(|e| e.to_string())?;
    rekindle_files::assemble_attachment(adapter.as_ref(), community_id, &fetched)
        .map_err(|e| e.to_string())
}

/// The stored attachment record for `attachment_id_hex` in a channel, from
/// the message row that carries it.
pub async fn attachment_record(
    state: &SharedState,
    pool: &rekindle_db::Db,
    channel_id: &str,
    attachment_id_hex: &str,
) -> Result<Option<rekindle_files::AttachmentRecordJson>, String> {
    let owner = crate::state_helpers::current_owner_key(state)?;
    let channel = channel_id.to_owned();
    let wanted = attachment_id_hex.to_owned();
    crate::db_helpers::db_call(pool, move |conn| {
        let mut stmt = conn.prepare(
            "SELECT attachment_json FROM messages \
             WHERE owner_key = ?1 AND conversation_id = ?2 AND conversation_type = 'channel' \
             AND attachment_json IS NOT NULL",
        )?;
        let rows = stmt.query_map(rusqlite::params![owner, channel], |row| {
            rekindle_db::row::get_json::<rekindle_files::AttachmentRecordJson>(
                row,
                "attachment_json",
            )
        })?;
        for record in rows {
            let record = record?;
            if record.attachment_id == wanted {
                return Ok(Some(record));
            }
        }
        Ok(None)
    })
    .await
}
