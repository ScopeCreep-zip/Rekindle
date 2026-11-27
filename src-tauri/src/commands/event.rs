//! The per-window event stream. See `crate::event_router`.

use tauri::ipc::Channel;
use tauri::State;

use crate::event_router::OutboundEnvelope;
use crate::state::SharedState;

/// Open this window's event stream on `channel`.
///
/// `from` is `None` for a window that just opened: it hydrates its history
/// from the database, so only the session's standing events (a security
/// alert raised before it subscribed, plan E7.1) and the reviews the backend
/// holds (`heldReviews`) are handed to it. A window
/// that reloaded passes the last cursor it processed (plan D3.2) and
/// receives the journaled events after it that were addressed to it, then
/// `resumed`, then the same standing snapshot (the held reviews included,
/// plan E3.2 S7d-keys.10); or `invalidSession` when the host can no longer
/// serve the cursor, and it reloads. The window is identified by the webview that invoked the
/// command, so it cannot subscribe as another window.
#[tauri::command]
pub async fn subscribe_events(
    window: tauri::WebviewWindow,
    channel: Channel<OutboundEnvelope>,
    from: Option<rekindle_types::subscription_events::Cursor>,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    let label = window.label().to_owned();
    let snapshot = || match (state.host.current(), from) {
        (Some(host), None) => crate::event_snapshot::standing_for(&host, &label),
        (Some(host), Some(from)) => crate::event_snapshot::resume_for(&host, &label, from),
        (None, None) => Vec::new(),
        (None, Some(_)) => vec![OutboundEnvelope::invalid_session()],
    };
    state
        .event_router
        .register(label.clone(), channel, snapshot);
    Ok(())
}
