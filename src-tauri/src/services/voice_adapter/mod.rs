//! Phase 14 — voice session adapter.
//!
//! Implements `rekindle_voice::VoiceSessionDeps` against the live
//! `AppState` + `tauri::AppHandle` + `Db`. Every method maps a
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
//! * [`media_keys`] — the `MediaKeySource` half: SFrame key sources
//!   for calls and community channels.

use rekindle_types::subscription_events::{SubscriptionEvent, VoiceEvent};
use std::sync::atomic::Ordering;
use std::sync::Arc;

use rekindle_lifecycle::{ScopeClosed, SessionScope};
use rekindle_voice::{VoiceSessionDeps, VoiceShutdownOpts};

use crate::state::AppState;
use rekindle_db::Db;

pub mod deps_impl;
pub mod event_mapping;
pub mod frame_sender;
pub mod io_helpers;
pub mod media_keys;
pub mod session_setup;

pub struct VoiceAdapter {
    pub(super) state: Arc<AppState>,
    pub(super) app_handle: tauri::AppHandle,
    pub(super) pool: Db,
}

impl VoiceAdapter {
    #[must_use]
    pub fn new(state: Arc<AppState>, app_handle: tauri::AppHandle, pool: Db) -> Arc<Self> {
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
    let pool = state.db.current()?;
    let adapter = VoiceAdapter::new(state.clone(), app.clone(), pool);
    let deps: Arc<dyn VoiceSessionDeps> = adapter;
    rekindle_voice::session::start_session(&deps, channel_id, community_id)
        .await
        .map_err(|e| e.to_string())
}

/// Architecture §10.6 — directed advertise of our `MediaCapabilities`
/// to the active channel roster. Free-fn facade used by the voice
/// signaling adapter's `advertise_media_capabilities` deps method
/// (join-apply / roster-apply re-advertise hooks).
pub fn advertise_media_capabilities(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
) -> Result<(), String> {
    io_helpers::broadcast_media_capabilities_impl(state, community_id, channel_id)
}

/// Re-broadcast VoiceJoin carrying the refreshed route blob. Free-fn
/// facade for the route-refresh / dead-route recovery paths
/// (`services/veilid/network.rs`) — no-op when no community voice
/// session is bound.
pub fn reannounce_voice_route(state: &Arc<AppState>) {
    let Some(app_handle) = state.app_handle.read().clone() else {
        return;
    };
    let Ok(pool) = state.db.current() else {
        return;
    };
    let adapter = VoiceAdapter::new(Arc::clone(state), app_handle, pool);
    let deps: Arc<dyn VoiceSessionDeps> = adapter;
    rekindle_voice::session::reannounce_voice_route(&deps);
}

/// Tear down voice with the given scope. Wraps adapter + crate call.
/// Takes `&AppState` (not `&Arc<AppState>`) for caller compat — the
/// callers in cleanup.rs / message_service.rs have a borrow only.
pub async fn shutdown_voice(state: &AppState, opts: &VoiceShutdownOpts) {
    let Some(app_handle) = state.app_handle.read().clone() else {
        tracing::warn!("shutdown_voice: no app handle on state — falling back to direct teardown");
        return;
    };
    let Ok(pool) = state.db.current() else {
        return;
    };
    let Some(state_arc) =
        tauri::Manager::try_state::<Arc<AppState>>(&app_handle).map(|s| Arc::clone(s.inner()))
    else {
        return;
    };
    // Capture the active slot BEFORE teardown drops the engine handle
    // — the media-ready gate must flip not-ready for the session that
    // is ending.
    let active_slot = {
        let ve = state.voice_engine.lock();
        ve.as_ref()
            .and_then(|h| h.community_id.clone().map(|c| (c, h.channel_id.clone())))
    };
    let adapter = VoiceAdapter::new(Arc::clone(&state_arc), app_handle, pool);
    let deps: Arc<dyn VoiceSessionDeps> = adapter;
    rekindle_voice::session::shutdown_voice(&deps, opts).await;
    if let Some((community_id, channel_id)) = active_slot {
        crate::services::community::media_ready_runtime::clear_media_ready(
            &state_arc,
            &community_id,
            &channel_id,
        );
        // The channel session's keys end with it; a rejoin draws new ones
        // (plan C7.20).
        state_arc.voice_sender_keys.end(&community_id, &channel_id);
    }
    // Native camera session dies with the voice session — its frames
    // have nowhere to go without the pacer/roster below.
    crate::services::native_video::stop(state);
    // Phase 4 — the video pacer stopped with the voice loops' scope;
    // clear its channels so the next session starts a fresh one.
    *state.video_pacer_tx.write() = None;
    *state.video_pacer_rate_tx.write() = None;
    *state.video_payload_share_rx.write() = None;
    state.video_bitrate_targets.lock().clear();

    // Belt-and-suspenders: clear voice channels even if the adapter
    // path early-returned (no AppHandle in tests, etc.).
    *state.voice_packet_tx.write() = None;
    *state.voice_packet_rx_staged.lock() = None;
}

/// W14.4 — spawn the 1-second packet-drop telemetry poller. Emits
/// `VoiceEvent::PacketsDropped` when the counter is non-zero, then
/// resets. Runs on the login scope, started from `spawn_login_services`.
///
/// # Errors
/// [`ScopeClosed`] when the session already ended.
pub fn spawn_drop_telemetry(
    state: &Arc<AppState>,
    app: &tauri::AppHandle,
    scope: &Arc<SessionScope>,
) -> Result<(), ScopeClosed> {
    let task_state = state.clone();
    let task_app = app.clone();
    scope.spawn_with_token("voice drop telemetry", |stop| async move {
        loop {
            let tick = tokio::time::sleep(std::time::Duration::from_secs(1));
            if stop.run_until_cancelled(tick).await.is_none() {
                return;
            }
            let count = task_state.voice_pkt_drops.swap(0, Ordering::Relaxed);
            task_state
                .voice_ingress_drops_total
                .fetch_add(count, Ordering::Relaxed);
            // This loop lives at login scope, so it ticks before any
            // call starts. Drops belong to a session; with none running
            // there is nothing to attribute them to.
            if count > 0 {
                if let Some(scope) = crate::state_helpers::current_voice_scope(&task_state) {
                    crate::event_dispatch::emit_subscription(
                        &task_app,
                        &SubscriptionEvent::Voice(VoiceEvent::PacketsDropped {
                            scope,
                            reason: "voice_pkt_drops".into(),
                            count,
                        }),
                    );
                }
            }
        }
    })
}
