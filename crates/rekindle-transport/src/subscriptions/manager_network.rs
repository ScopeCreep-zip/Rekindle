//! Network-level callbacks: attachment changes, route deaths, and watch
//! re-establishment.

use tracing::{debug, info, warn};

use super::events::{self, SubscriptionEvent};
use super::{watches, SubscriptionManager};

impl SubscriptionManager {
    /// Handle attachment state change.
    pub async fn on_attachment_change(&self, is_attached: bool, public_internet_ready: bool) {
        self.process_event(SubscriptionEvent::Network(
            events::NetworkEvent::AttachmentChanged {
                is_attached,
                public_internet_ready,
            },
        ));

        // Re-establish all watches on re-attach
        if is_attached && public_internet_ready {
            info!("network re-attached — re-establishing all watches");
            let stale = self.watches.read().needs_renewal();
            for (record_key, entry) in stale {
                if watches::renew_watch(&self.node, &record_key, &entry.subkeys).await {
                    if let Some(e) = self.watches.write().entries.get_mut(&record_key) {
                        e.established_at = std::time::Instant::now();
                    }
                }
            }
        }
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

    /// Handle a watch death (count=0 or empty subkeys).
    pub async fn on_watch_died(&self, record_key: &str) {
        let entry = self.watches.read().get(record_key).cloned();
        if let Some(entry) = entry {
            info!(record_key, "watch died — re-establishing");
            if watches::renew_watch(&self.node, record_key, &entry.subkeys).await {
                if let Some(e) = self.watches.write().entries.get_mut(record_key) {
                    e.established_at = std::time::Instant::now();
                }
                self.process_event(SubscriptionEvent::Network(
                    events::NetworkEvent::WatchReestablished {
                        record_key: record_key.into(),
                    },
                ));
            } else {
                warn!(record_key, "watch re-establishment failed");
                self.process_event(SubscriptionEvent::Network(
                    events::NetworkEvent::WatchFailed {
                        record_key: record_key.into(),
                        error: "re-establishment failed after watch death".into(),
                    },
                ));
            }
        } else {
            debug!(record_key, "watch died for unregistered record — ignoring");
        }
    }
}
