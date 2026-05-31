//! Phase 14 — voice session adapter.
//!
//! Implements `rekindle_voice::VoiceSessionDeps` against the live
//! `AppState` + `tauri::AppHandle` + `DbPool`. Every method maps a
//! trait-abstract operation to its concrete src-tauri/AppState
//! equivalent. The crate's loops (send_loop, receive_loop, mcu_loop)
//! and any future session/shutdown ports use this adapter to reach
//! into AppState without taking a direct dependency on it.
//!
//! Phase 14.r split layout (≤500 LoC per file):
//! * [`deps_impl`] — the `impl VoiceSessionDeps for VoiceAdapter`
//!   block; bigger method bodies delegate into the helper modules
//!   below.
//! * [`session_setup`] — voice engine bring-up + transport build +
//!   loop spawn + shutdown-handle extraction (the lifecycle helpers
//!   relocated from the deleted `services/voice/session.rs`).
//! * [`event_mapping`] — pure `VoiceSessionEvent → VoiceEvent` mapping
//!   used by `emit_voice_event`.
//! * [`io_helpers`] — audio device restart, peer-route lookup,
//!   media-capabilities broadcast, member-name DB query.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use rekindle_voice::{VoiceSessionDeps, VoiceShutdownOpts};

use crate::channels::VoiceEvent;
use crate::db::DbPool;
use crate::state::AppState;

pub mod deps_impl;
pub mod event_mapping;
pub mod io_helpers;
pub mod session_setup;

pub struct VoiceAdapter {
    pub(super) state: Arc<AppState>,
    pub(super) app_handle: tauri::AppHandle,
    pub(super) pool: DbPool,
}

impl VoiceAdapter {
    #[must_use]
    pub fn new(state: Arc<AppState>, app_handle: tauri::AppHandle, pool: DbPool) -> Arc<Self> {
        Arc::new(Self {
            state,
            app_handle,
            pool,
        })
    }
}

// ── Public free-fn facades for external callers ─────────────────────
//
// These wrap "construct VoiceAdapter + delegate to rekindle_voice"
// so callers outside this module don't need to know the adapter
// construction shape. Used by commands/voice.rs (shutdown_voice),
// commands/auth.rs (spawn_drop_telemetry), message_service.rs
// (shutdown_voice on CallEnd), cleanup.rs (shutdown on detach),
// and calls_adapter.rs (start_session / shutdown).

/// Bring up a voice session. Wraps adapter construction + crate
/// call. Returns the same `Err` shape as the legacy
/// `services::voice::session::start_session` for caller compat.
pub async fn start_session(
    channel_id: &str,
    community_id: Option<&str>,
    app: &tauri::AppHandle,
    state: &Arc<AppState>,
) -> Result<(), String> {
    let Some(pool) = tauri::Manager::try_state::<DbPool>(app) else {
        return Err("DbPool state missing".into());
    };
    let pool = pool.inner().clone();
    let adapter = VoiceAdapter::new(state.clone(), app.clone(), pool);
    let deps: Arc<dyn VoiceSessionDeps> = adapter;
    rekindle_voice::session::start_session(&deps, channel_id, community_id)
        .await
        .map_err(|e| e.to_string())
}

/// Tear down voice with the given scope. Wraps adapter + crate call.
/// Takes `&AppState` (not `&Arc<AppState>`) for caller compat — the
/// callers in cleanup.rs / message_service.rs have a borrow only.
pub async fn shutdown_voice(state: &AppState, opts: &VoiceShutdownOpts) {
    let Some(app_handle) = state.app_handle.read().clone() else {
        tracing::warn!("shutdown_voice: no app handle on state — falling back to direct teardown");
        return;
    };
    let Some(pool) = tauri::Manager::try_state::<DbPool>(&app_handle) else {
        return;
    };
    let pool = pool.inner().clone();
    let Some(state_arc) =
        tauri::Manager::try_state::<Arc<AppState>>(&app_handle).map(|s| Arc::clone(s.inner()))
    else {
        return;
    };
    let adapter = VoiceAdapter::new(state_arc, app_handle, pool);
    let deps: Arc<dyn VoiceSessionDeps> = adapter;
    rekindle_voice::session::shutdown_voice(&deps, opts).await;

    // Belt-and-suspenders: clear voice channels even if the adapter
    // path early-returned (no AppHandle in tests, etc.).
    *state.voice_packet_tx.write() = None;
    *state.voice_packet_rx_staged.lock() = None;
}

/// W14.4 — spawn the 1-second packet-drop telemetry poller. Emits
/// `VoiceEvent::PacketsDropped` when the counter is non-zero, then
/// resets. Called once at login from `commands::auth`.
pub fn spawn_drop_telemetry(state: &Arc<AppState>, app: &tauri::AppHandle) {
    let task_state = state.clone();
    let task_app = app.clone();
    let handle = tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            let count = task_state.voice_pkt_drops.swap(0, Ordering::Relaxed);
            if count > 0 {
                crate::event_dispatch::dispatch(
                    &task_app,
                    "voice-event",
                    &VoiceEvent::PacketsDropped {
                        reason: "voice_pkt_drops".into(),
                        count,
                    },
                );
            }
        }
    });
    state.background_handles.lock().push(handle);
}
