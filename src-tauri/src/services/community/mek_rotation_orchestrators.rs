//! Phase 23.D.11 — thin facade. The rotator-initiated rotation
//! (rotate_text_mek_for_departure) is ported into `rekindle_mek_rotation::rotate` parameterised over
//! `MekDistributeDeps`. The `RequestMEK` responder is the Host's
//! (`HostGovernance::answer_mek_request`, plan E3.2 S6).

use std::sync::Arc;

use crate::services::mek_adapter::MekAdapter;
use crate::state::AppState;

fn build_adapter(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
) -> Result<Arc<MekAdapter>, String> {
    let pool = state.db.current()?;
    Ok(MekAdapter::new(Arc::clone(state), app_handle.clone(), pool))
}

pub async fn rotate_text_mek_for_departure(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    departed_pseudonym: &str,
) -> Result<(), String> {
    let adapter = build_adapter(app_handle, state)?;
    rekindle_mek_rotation::rotate_text_mek_for_departure(
        adapter.as_ref(),
        community_id,
        departed_pseudonym,
    )
    .await
    .map_err(|e| e.to_string())
}

/// A member left the community — kicked, banned or departed: replace the
/// keys they hold. The community key (plan D20; architecture §10.5), and
/// our call-media key when they share our voice channel (plan C7.20).
/// Fire-and-forget: the removal itself has already happened and must not
/// wait on distribution.
pub fn spawn_departure_rotations(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    departed_pseudonym: &str,
) {
    crate::services::voice_signaling_adapter::member_departed(
        app_handle,
        state,
        community_id,
        departed_pseudonym,
    );
    let app_handle = app_handle.clone();
    let state = Arc::clone(state);
    let community_id = community_id.to_string();
    let departed = departed_pseudonym.to_string();
    crate::state_helpers::login_scope_or_closed(&state).spawn_or_drop("departure MEK rotation", async move {
        if let Err(error) =
            rotate_text_mek_for_departure(&app_handle, &state, &community_id, &departed).await
        {
            tracing::debug!(community = %community_id, member = %departed, %error, "community MEK rotation skipped after departure");
        }
    });
}
