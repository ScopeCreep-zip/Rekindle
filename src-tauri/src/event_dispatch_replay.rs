//! Journal replay and standing events for windows as they subscribe.

use super::{OutboundEnvelope, WebviewEvent};

/// A reloading window's replay from `from` (plan D3.2): the journaled
/// events after it that were addressed to `label`, then `Resumed`; or
/// `InvalidSession`, when the host cannot serve the cursor.
#[must_use]
pub fn replay_for(
    host: &rekindle_node::host::Host,
    label: &str,
    from: rekindle_types::subscription_events::Cursor,
) -> Vec<OutboundEnvelope> {
    use rekindle_types::subscription_events::StreamFrame;
    host.replay(from)
        .into_iter()
        .filter_map(|frame| match frame {
            StreamFrame::Event(sequenced) => {
                let event = WebviewEvent::Subscription(sequenced.event);
                event
                    .audience()
                    .includes(label)
                    .then(|| event.envelope(Some(sequenced.cursor)))
                    .flatten()
            }
            StreamFrame::Resumed(cursor) => Some(OutboundEnvelope::resumed(cursor)),
            StreamFrame::InvalidSession => Some(OutboundEnvelope::invalid_session()),
            // Never journaled, so a replay never holds it.
            StreamFrame::HostStopping { .. } => None,
        })
        .collect()
}

/// The session's standing events addressed to `label`, with their cursors
/// (plan E7.1): what a window that just opened is handed before its live
/// events, so a security alert raised at unlock, before the window
/// subscribed, still reaches it.
#[must_use]
pub fn standing_for(host: &rekindle_node::host::Host, label: &str) -> Vec<OutboundEnvelope> {
    host.standing()
        .into_iter()
        .filter_map(|sequenced| {
            let event = WebviewEvent::Subscription(sequenced.event);
            event
                .audience()
                .includes(label)
                .then(|| event.envelope(Some(sequenced.cursor)))
                .flatten()
        })
        .collect()
}
