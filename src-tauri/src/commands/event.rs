//! The per-window event stream. See `crate::event_router`.

use tauri::ipc::Channel;
use tauri::State;

use crate::event_router::OutboundEnvelope;
use crate::state::SharedState;

/// Open this window's event stream on `channel`.
///
/// `from` is `None` for a window that just opened: it hydrates its history
/// from the database, so only the session's standing events (a security
/// alert raised before it subscribed, plan E7.1) are handed to it. A window
/// that reloaded passes the last cursor it processed (plan D3.2) and
/// receives the journaled events after it that were addressed to it, then
/// `resumed`; or `invalidSession` when the host can no longer serve the
/// cursor, and it reloads. The window is identified by the webview that invoked the
/// command, so it cannot subscribe as another window.
#[tauri::command]
pub async fn subscribe_events(
    window: tauri::WebviewWindow,
    channel: Channel<OutboundEnvelope>,
    from: Option<rekindle_types::subscription_events::Cursor>,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    let label = window.label().to_owned();
    let host = state.host.current();
    let Some(from) = from else {
        // Read under the router's lock, inside `register`: an alert
        // published after this snapshot is delivered live once the window
        // is registered, and one in both is processed once (by its cursor).
        let window = label.clone();
        let snapshot = std::iter::once(()).flat_map(move |()| {
            host.as_ref()
                .map(|host| crate::event_snapshot::standing_for(host, &window))
                .unwrap_or_default()
        });
        state.event_router.register(label, channel, snapshot);
        return Ok(());
    };
    let replay = match host {
        Some(host) => crate::event_snapshot::replay_for(&host, &label, from),
        // Before the host starts there is no journal to replay from.
        None => vec![OutboundEnvelope::invalid_session()],
    };
    state.event_router.register(label, channel, replay);
    Ok(())
}
