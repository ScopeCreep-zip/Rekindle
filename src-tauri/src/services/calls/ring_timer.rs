//! Wave 13 W13.2 — per-call_id 30 s ring timeout.
//!
//! Each side enforces locally (no cross-host time sync). When the
//! timer fires, if the CallState is still Outgoing or Incoming, drop
//! the entry and emit the appropriate frontend event:
//!
//!   - Outgoing → ChatEvent::CallTimedOut + persist missed_call (caller side)
//!   - Incoming → ChatEvent::CallMissed   + persist missed_call (receiver side)
//!
//! Cancellation is implicit: if the call transitions out of
//! Outgoing/Incoming (Connecting/Active/dropped) before the timer
//! fires, the timer's post-sleep state check sees the gone-or-different
//! state and exits without emitting. No explicit cancel channel — keeps
//! the spawn surface tiny.
//!
//! Replaces the inline timer in the old `commands::calls::start_dm_call`
//! and the parked-oneshot timer in `handle_incoming_offer`.

use std::time::Duration;

use rekindle_calls::{CallKind, CallStatus};
use tauri::Emitter;

use crate::channels::ChatEvent;
use crate::db::DbPool;
use crate::state::SharedState;

/// Caller-side: spawn a tokio task that fires `CallTimedOut` if the
/// call is still Outgoing at `expires_at_ms`.
pub fn spawn_dialing_timeout(
    state: &SharedState,
    pool: DbPool,
    app: tauri::AppHandle,
    call_id: String,
    peer_key: String,
    kind: CallKind,
    expires_at_ms: u64,
) {
    let task_state = state.clone();
    let handle = tauri::async_runtime::spawn(async move {
        let now = rekindle_utils::timestamp_ms();
        let remaining = expires_at_ms.saturating_sub(now);
        tokio::time::sleep(Duration::from_millis(remaining.max(1))).await;

        let still_dialing = {
            let calls = task_state.active_calls.lock();
            calls
                .get(&call_id)
                .is_some_and(|c| matches!(c.status, CallStatus::Outgoing))
        };
        if !still_dialing {
            return;
        }
        task_state.active_calls.lock().remove(&call_id);
        super::persist_missed_call(&pool, &task_state, &call_id, &peer_key, kind, expires_at_ms);
        let _ = app.emit(
            "chat-event",
            &ChatEvent::CallTimedOut {
                call_id: call_id.clone(),
            },
        );
    });
    state.background_handles.lock().push(handle);
}

/// Receiver-side: spawn a tokio task that fires `CallMissed` if the
/// call is still Incoming at `expires_at_ms`.
pub fn spawn_incoming_timeout(
    state: &SharedState,
    pool: DbPool,
    app: tauri::AppHandle,
    call_id: String,
    peer_key: String,
    kind: CallKind,
    expires_at_ms: u64,
) {
    let task_state = state.clone();
    let handle = tauri::async_runtime::spawn(async move {
        let now = rekindle_utils::timestamp_ms();
        let remaining = expires_at_ms.saturating_sub(now);
        tokio::time::sleep(Duration::from_millis(remaining.max(1))).await;

        let still_incoming = {
            let calls = task_state.active_calls.lock();
            calls
                .get(&call_id)
                .is_some_and(|c| matches!(c.status, CallStatus::Incoming))
        };
        if !still_incoming {
            return;
        }
        task_state.active_calls.lock().remove(&call_id);
        super::persist_missed_call(&pool, &task_state, &call_id, &peer_key, kind, expires_at_ms);
        let _ = app.emit(
            "chat-event",
            &ChatEvent::CallMissed {
                call_id: call_id.clone(),
                from: peer_key,
            },
        );
    });
    state.background_handles.lock().push(handle);
}
