//! What a window is handed when it subscribes (plan D3.2, E7.1, E3.2
//! S7d-keys.10): the standing snapshot (the session's standing events and
//! the backend's held reviews) on a first subscribe, and on a resume the
//! replay from its cursor followed by that same snapshot, so a review held
//! or an alert raised while it was away reaches it either way.

use crate::event_dispatch::WebviewEvent;
use crate::event_router::OutboundEnvelope;

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
                let event = crate::event_dispatch::WebviewEvent::Subscription(sequenced.event);
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
/// (plan E7.1), then the complete set of reviews the backend holds
/// (`heldReviews`, no cursor, empty included): what a window that just
/// opened is handed before its live events, so a security alert raised at
/// unlock, or a review held, before the window subscribed still reaches
/// it. Every envelope is marked `standing`.
#[must_use]
pub fn standing_for(host: &rekindle_node::host::Host, label: &str) -> Vec<OutboundEnvelope> {
    host.standing()
        .into_iter()
        .map(|sequenced| (Some(sequenced.cursor), sequenced.event))
        .chain(std::iter::once((None, host.held_reviews())))
        .filter_map(|(cursor, event)| {
            let event = WebviewEvent::Subscription(event);
            event
                .audience()
                .includes(label)
                .then(|| event.envelope(cursor))
                .flatten()
                .map(OutboundEnvelope::into_standing)
        })
        .collect()
}

/// A reloading window's resume from `from`: [`replay_for`], then
/// [`standing_for`], whose held set replaces the one the window had (a
/// review resolved while it was away arrives as `heldReviews: []`).
#[must_use]
pub fn resume_for(
    host: &rekindle_node::host::Host,
    label: &str,
    from: rekindle_types::subscription_events::Cursor,
) -> Vec<OutboundEnvelope> {
    let mut envelopes = replay_for(host, label, from);
    envelopes.extend(standing_for(host, label));
    envelopes
}

#[cfg(test)]
#[path = "event_snapshot_tests.rs"]
mod tests;
