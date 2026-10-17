//! The in-process host (plan D3.1), set once `Host::start` has finished:
//! the GUI starts it after its node, so a command that needs it before then
//! waits for it.
//!
//! The GUI's subscription events are journaled by the host (plan D3.2).
//! Those emitted before it starts (the node's first attachment changes)
//! wait in order in a bounded queue and are published when it starts.

use std::collections::VecDeque;
use std::sync::Arc;

use rekindle_node::host::Host;
use rekindle_types::subscription_events::SubscriptionEvent;

/// How many events wait for the host before the oldest is dropped.
const BEFORE_START_CAPACITY: usize = 1024;

/// Holds the host once it has started.
pub struct HostSlot {
    tx: tokio::sync::watch::Sender<Option<Arc<Host>>>,
    before_start: parking_lot::Mutex<VecDeque<SubscriptionEvent>>,
}

impl Default for HostSlot {
    fn default() -> Self {
        Self {
            tx: tokio::sync::watch::Sender::new(None),
            before_start: parking_lot::Mutex::new(VecDeque::new()),
        }
    }
}

impl HostSlot {
    /// Install the started host and publish what waited for it.
    pub fn set(&self, host: Arc<Host>) {
        let mut waiting = self.before_start.lock();
        for event in waiting.drain(..) {
            host.publish(event);
        }
        self.tx.send_replace(Some(host));
    }

    /// Journal `event` in the host, or queue it until the host starts.
    pub fn publish(&self, event: SubscriptionEvent) {
        if let Some(host) = self.current() {
            host.publish(event);
            return;
        }
        let mut waiting = self.before_start.lock();
        // The host may have started while this waited for the lock.
        if let Some(host) = self.current() {
            host.publish(event);
            return;
        }
        if waiting.len() >= BEFORE_START_CAPACITY {
            tracing::warn!("event dropped while the host was starting");
            waiting.pop_front();
        }
        waiting.push_back(event);
    }

    /// The host, waiting until it has started.
    pub async fn get(&self) -> Arc<Host> {
        let mut rx = self.tx.subscribe();
        loop {
            if let Some(host) = rx.borrow_and_update().clone() {
                return host;
            }
            // The sender lives in `AppState`, so it is never dropped while
            // a command runs.
            if rx.changed().await.is_err() {
                std::future::pending::<()>().await;
            }
        }
    }

    /// The host, if it has started.
    pub fn current(&self) -> Option<Arc<Host>> {
        self.tx.borrow().clone()
    }
}
