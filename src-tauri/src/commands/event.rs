//! Phase 10 — `event_resume` Tauri command.
//!
//! Frontend persists the last seen cursor to `localStorage`. On every
//! window mount it calls `event_resume(last_cursor)`; the backend
//! re-broadcasts any journal entries with `cursor > effective_cursor`
//! (where `effective_cursor = max(replay_watermark, last_cursor)`) so
//! every webview's live `safeListen` handlers process them as if they
//! arrived in real time.

use tauri::State;

use crate::state::SharedState;

#[tauri::command]
pub async fn event_resume(state: State<'_, SharedState>, last_cursor: u64) -> Result<u64, String> {
    Ok(crate::services::event_resume_runtime::event_resume_inner(
        state.inner(),
        last_cursor,
    ))
}
