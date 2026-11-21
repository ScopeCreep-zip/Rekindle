//! A 1:1 call's media and invite row, for `CallsAdapter` (plans CE2, CE3;
//! lead decision 30): the voice session is owned by one call id
//! (`AppState::call_media`), and a call's invite row is settled by its answer.

use std::sync::Arc;

use rekindle_calls::CallError;

use crate::state::AppState;

/// Start `call_id`'s voice session to `peer_pubkey_hex`. One session runs,
/// owned by one call: another call's media still running is torn down
/// before this call's starts (the FULL teardown clears its ownership).
pub(super) async fn start(
    state: &Arc<AppState>,
    app_handle: &tauri::AppHandle,
    call_id: &str,
    peer_pubkey_hex: &str,
) -> Result<(), CallError> {
    if let Some(previous) = state.call_media.owner().filter(|o| o != call_id) {
        tracing::info!(call = %call_id, %previous,
            "another call's media is still running; tearing it down first");
        crate::services::voice_adapter::shutdown_voice(
            state,
            &rekindle_voice::VoiceShutdownOpts::FULL,
        )
        .await;
    }
    // Owned before the session starts: its receive loop reads the call's
    // SFrame keys by this owner (`MediaKeySource::call_media`). The crate
    // handler stored the call's media secret in the registry already.
    state.call_media.claim(call_id);
    // 1:1 calls pass the peer as the channel-id argument (`community_id`
    // is `None` for 1:1).
    let started =
        crate::services::voice_adapter::start_session(peer_pubkey_hex, None, app_handle, state)
            .await;
    if started.is_err() {
        state.call_media.release(call_id);
    }
    started.map_err(|e| CallError::Session(format!("voice session start: {e}")))
}

/// Stop `call_id`'s media; a no-op when another call's media runs, or none:
/// ending one call never stops another's.
pub(super) async fn shutdown(state: &Arc<AppState>, call_id: &str) {
    if !state.call_media.release(call_id) {
        tracing::debug!(call = %call_id, "call's media is not running; nothing to stop");
        return;
    }
    crate::services::voice_adapter::shutdown_voice(state, &rekindle_voice::VoiceShutdownOpts::FULL)
        .await;
}

/// Settle `call_id`'s invite row (`answered`, or discarded). A storage
/// failure is logged: the row then fails at its deadline, the invite's
/// expiry.
pub(super) async fn settle_invite(state: &Arc<AppState>, call_id: &str, answered: bool) {
    let host = match crate::state_helpers::friend_host(state) {
        Ok(host) => host,
        Err(e) => {
            tracing::debug!(call = %call_id, error = %e, "invite row not settled: no host");
            return;
        }
    };
    let settled =
        rekindle_transport::operations::dm::call_invite::settle(host.as_ref(), call_id, answered)
            .await;
    if let Err(e) = settled {
        tracing::warn!(call = %call_id, error = %e,
            "invite row not settled; it fails at the invite's expiry");
    }
}
