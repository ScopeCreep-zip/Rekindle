//! The in-process host (plan D3.1), set once `Host::start` has finished:
//! the GUI starts it after its node, so a command that needs it before then
//! waits for it.

use std::sync::Arc;

use rekindle_node::host::Host;

/// Holds the host once it has started.
pub struct HostSlot {
    tx: tokio::sync::watch::Sender<Option<Arc<Host>>>,
}

impl Default for HostSlot {
    fn default() -> Self {
        Self {
            tx: tokio::sync::watch::Sender::new(None),
        }
    }
}

impl HostSlot {
    /// Install the started host.
    pub fn set(&self, host: Arc<Host>) {
        self.tx.send_replace(Some(host));
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
