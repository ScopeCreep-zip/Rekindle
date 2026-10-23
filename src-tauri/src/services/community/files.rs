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
//! The upload, download, voice message and audio are the Host's (plan E3.1
//! E3d, `PinRequest`); this module keeps the pinned-set sync the shell's
//! governance merge runs.

use crate::state::SharedState;

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
