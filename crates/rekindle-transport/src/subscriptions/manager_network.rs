//! Network-level callbacks: attachment changes and route deaths. Watch
//! death is the record pool's (plan C7.8).

use super::events::{self, SubscriptionEvent};
use super::SubscriptionManager;

impl SubscriptionManager {
    /// Handle attachment state change.
    ///
    /// `attachment_state` is the raw Veilid string that
    /// [`TransportEvent::AttachmentChanged`](crate::handler::TransportEvent)
    /// already carries, and `has_route` comes from the caller because
    /// route state lives on the broadcast side, not on this manager.
    /// Both are needed for the event to say anything a status indicator
    /// can use: "attaching" and "attached_weak" both read as not-ready
    /// through the booleans alone, and being attached with no route
    /// means nobody can reach us — invisible from the other fields.
    pub fn on_attachment_change(
        &self,
        attachment_state: String,
        is_attached: bool,
        public_internet_ready: bool,
        has_route: bool,
        media_route: events::RouteAvailability,
    ) {
        self.process_event(SubscriptionEvent::Network(
            events::NetworkEvent::AttachmentChanged {
                attachment_state,
                is_attached,
                public_internet_ready,
                has_route,
                media_route,
            },
        ));
    }

    /// Handle route deaths.
    pub fn on_route_change(&self, local_died: usize, remote_died: Vec<String>) {
        if local_died > 0 {
            self.process_event(SubscriptionEvent::Network(
                events::NetworkEvent::LocalRoutesDied { count: local_died },
            ));
        }
        if !remote_died.is_empty() {
            self.process_event(SubscriptionEvent::Network(
                events::NetworkEvent::RemoteRoutesDied {
                    peer_keys: remote_died,
                },
            ));
        }
    }
}
