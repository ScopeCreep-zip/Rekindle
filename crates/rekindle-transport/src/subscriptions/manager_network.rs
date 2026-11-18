//! Network-level callbacks: attachment changes and route deaths. Watch
//! death is the record pool's (plan C7.8).

use super::events::{self, SubscriptionEvent};
use super::SubscriptionManager;

impl SubscriptionManager {
    /// Handle attachment state change: `status` is the
    /// [`NetworkEvent::AttachmentChanged`](events::NetworkEvent) the caller
    /// built, because its route facts live on the broadcast side and its
    /// `can_write` on the Host's lifecycle, neither of which this manager
    /// holds. It carries the raw Veilid state string, since "attaching" and
    /// "attached_weak" both read as not-ready through the booleans alone,
    /// and being attached with no route means nobody can reach us.
    pub fn on_attachment_change(&self, status: events::NetworkEvent) {
        self.process_event(SubscriptionEvent::Network(status));
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
