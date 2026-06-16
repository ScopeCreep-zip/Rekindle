//! Reconnection state invalidation — clears connection-scoped state on disconnect.

use super::effects::Effect;
use super::state::ephemeral::{RailSignal, SignalPriority, SignalScope, ToastLevel};
use super::state::in_flight::PendingSend;
use super::state::presence::PresenceState;
use super::state::TuiState;

pub fn begin(state: &mut TuiState) -> Vec<Effect> {
    tracing::warn!(
        dm_threads = state.dm.threads.len(),
        "tui: reconnect::begin — invalidating connection-scoped state"
    );
    invalidate(state);
    state.ephemeral.rails.set(RailSignal {
        id: "system:daemon_disconnected".into(),
        scope: SignalScope::System,
        text: "Daemon disconnected \u{2014} reconnecting...".into(),
        priority: SignalPriority::Critical,
        dismissible: false,
    });
    state.ephemeral.toasts.push(
        "Daemon disconnected \u{2014} reconnecting...".into(),
        ToastLevel::Warning,
        state.now,
    );
    vec![]
}

fn invalidate(state: &mut TuiState) {
    for (_, pending) in state.in_flight.drain_pending_sends() {
        match pending {
            PendingSend::Dm { peer_key, .. } => {
                if let Some(thread) = state.dm.threads.get_mut(&peer_key) {
                    thread.fail_last_sending();
                }
            }
            PendingSend::ChannelMessage { community, channel, body, reply_to } => {
                let key = super::state::channels::ChannelKey { community, channel };
                if let Some(ch) = state.channels.channels.get_mut(&key) {
                    ch.fail_by_body(&body, reply_to);
                }
            }
        }
    }

    state.in_flight.clear_all();
    state.ephemeral.clear_connection_scoped();

    for (_, thread) in state.dm.threads.iter_mut() {
        thread.is_typing = false;
        thread.typing_since = None;
    }

    state.presence = PresenceState::new();

    state.dm.inbox_loaded = false;
    state.dm.inbox_loaded_at = None;
    state.friends.loaded = false;
    state.friends.loaded_at = None;
    state.communities.list_loaded = false;
    state.communities.list_loaded_at = None;

    for (_, ch) in state.channels.channels.iter_mut() {
        ch.loaded = false;
        ch.loaded_at = None;
    }

    for (_, thread) in state.dm.threads.iter_mut() {
        thread.loaded = false;
    }

    state.node_connected = false;
}
