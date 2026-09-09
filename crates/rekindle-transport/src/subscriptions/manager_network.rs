//! Network-level callbacks: attachment changes, route deaths, and watch
//! re-establishment.

use std::sync::Arc;

use tracing::{debug, info, warn};

use super::events::{self, SubscriptionEvent};
use super::{watches, SubscriptionManager};

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
    pub async fn on_attachment_change(
        &self,
        attachment_state: String,
        is_attached: bool,
        public_internet_ready: bool,
        has_route: bool,
    ) {
        self.process_event(SubscriptionEvent::Network(
            events::NetworkEvent::AttachmentChanged {
                attachment_state,
                is_attached,
                public_internet_ready,
                has_route,
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

    /// Handle a watch death (`count == 0` or an empty subkey range).
    ///
    /// This is the **only** signal that a watch failed.
    /// `watch_dht_values` "records the desired watch state and returns
    /// without a network round-trip; a background task reconciles it
    /// with a remote node", and "no network errors surface here" — so
    /// its `bool` cannot report whether a watch was granted, and a
    /// record whose watch was refused for want of a slot looks
    /// identical at establish time to one that succeeded.
    ///
    /// Re-establishing immediately rather than waiting for the renewal
    /// loop: that runs on a 4-minute cadence, and PATH 3 is the
    /// consistency path — four minutes blind on a record we were asked
    /// to watch is the gap `inspect` exists to close, not one to open.
    ///
    /// Spawns rather than awaiting so the caller can hold a
    /// `parking_lot` guard while calling it; those are `!Send`.
    pub fn on_watch_died(&self, record_key: &str) {
        let Some(entry) = self.watches.read().get(record_key).cloned() else {
            debug!(record_key, "watch died for unregistered record — ignoring");
            return;
        };
        let node = Arc::clone(&self.node);
        let watches_reg = Arc::clone(&self.watches);
        let event_tx = self.event_tx.clone();
        let key = record_key.to_string();

        tokio::spawn(async move {
            info!(record_key = %key, "watch died — re-establishing");
            let reestablished = watches::renew_watch(&node, &key, &entry.subkeys).await;
            if reestablished {
                if let Some(e) = watches_reg.write().entries.get_mut(&key) {
                    e.established_at = std::time::Instant::now();
                }
            }
            // Emitted directly rather than through `process_event`: the
            // enrichment and state-effect stages have nothing to add to
            // a network event, and routing through them would need the
            // whole manager `'static`.
            let event = if reestablished {
                SubscriptionEvent::Network(events::NetworkEvent::WatchReestablished {
                    record_key: key,
                })
            } else {
                warn!(record_key = %key, "watch re-establishment failed");
                SubscriptionEvent::Network(events::NetworkEvent::WatchFailed {
                    record_key: key,
                    error: "re-establishment failed after watch death".into(),
                })
            };
            let _ = event_tx.send(event);
        });
    }
}
