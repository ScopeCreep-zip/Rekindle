//! The per-window event stream. See `crate::event_router`.

use tauri::ipc::Channel;
use tauri::State;

use crate::event_router::OutboundEnvelope;
use crate::state::SharedState;

/// Open this window's event stream on `channel`.
///
/// `from` is `None` for a window that just opened: it hydrates its history
/// from the database, so nothing is replayed. A window that reloaded passes
/// the last cursor it processed (plan D3.2) and receives the journaled
/// events after it that were addressed to it, then `resumed`; or
/// `invalidSession` when the host can no longer serve the cursor, and it
/// reloads. The window is identified by the webview that invoked the
/// command, so it cannot subscribe as another window.
#[tauri::command]
pub async fn subscribe_events(
    window: tauri::WebviewWindow,
    channel: Channel<OutboundEnvelope>,
    from: Option<rekindle_types::subscription_events::Cursor>,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    let label = window.label().to_owned();
    let replay = match (from, state.host.current()) {
        (Some(from), Some(host)) => crate::event_dispatch::replay_for(&host, &label, from),
        // Before the host starts there is no journal to replay from.
        (Some(_), None) => vec![OutboundEnvelope::invalid_session()],
        (None, _) => Vec::new(),
    };
    state.event_router.register(label, channel, replay);
    Ok(())
}
