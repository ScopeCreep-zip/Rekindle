//! The per-window event stream. See `crate::event_router`.

use tauri::ipc::Channel;
use tauri::State;

use crate::event_router::OutboundEnvelope;
use crate::state::SharedState;

/// Open this window's event stream on `channel`.
///
/// `last_seq` is `None` for a window that just opened: it hydrates its
/// history from the database, so nothing is replayed. A window that
/// reloaded passes the last sequence number it saw and receives the
/// journaled events after it that were addressed to it. The window is
/// identified by the webview that invoked the command, so it cannot
/// subscribe as another window.
#[tauri::command]
pub async fn subscribe_events(
    window: tauri::WebviewWindow,
    channel: Channel<OutboundEnvelope>,
    last_seq: Option<u64>,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    let label = window.label().to_owned();
    let replay = last_seq
        .map(|since| crate::event_dispatch::replay_for(&state, &label, since))
        .unwrap_or_default();
    state.event_router.register(label, channel, replay);
    Ok(())
}
