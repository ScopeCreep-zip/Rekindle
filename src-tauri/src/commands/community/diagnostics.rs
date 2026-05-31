use tauri::State;

use crate::state::SharedState;

use crate::services::community_diagnostics_runtime::GossipDiagnostics;

#[tauri::command]
pub async fn debug_gossip_state(
    state: State<'_, SharedState>,
    community_id: String,
) -> Result<GossipDiagnostics, String> {
    crate::services::community_diagnostics_runtime::debug_gossip_state_inner(
        state.inner(),
        community_id,
    )
}
