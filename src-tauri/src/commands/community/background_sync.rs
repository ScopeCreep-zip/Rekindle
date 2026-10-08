//! On-demand Tier 2 background fetch (architecture §17.3). Frontend
//! calls this from the resume-from-suspend hook or a manual "refresh"
//! button so any subkey writes that arrived while the app was idle
//! get picked up without waiting for the 60-second inspect interval.

use tauri::State;

use crate::services::community::background_sync::{run_background_sync_now, BackgroundSyncReport};
use crate::state::SharedState;

#[tauri::command]
pub async fn run_background_sync(
    state: State<'_, SharedState>,
) -> Result<BackgroundSyncReport, String> {
    run_background_sync_now(state.inner()).await
}
