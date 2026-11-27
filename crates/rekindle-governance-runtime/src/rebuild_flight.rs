//! A community's governance rebuilds coalesce into a dirty-generation
//! single flight (plan E3.2 S7l; D15: the state is a pure function of the
//! op set, so a rebuild whose op-set read came after a trigger already
//! covers that trigger). A trigger takes a ticket; the rebuild holding the
//! lock records the last ticket issued before it reads the op set, and a
//! waiting trigger whose ticket that rebuild covered returns without
//! rebuilding. A trigger while a rebuild runs therefore costs one more
//! rebuild after it, not one rebuild per trigger.

use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};

/// The trigger and build generations of one community's rebuilds.
#[derive(Debug, Default)]
pub struct RebuildFlight {
    /// Tickets issued, one per trigger.
    requested: AtomicU64,
    /// The highest ticket a finished rebuild covered.
    built: AtomicU64,
}

impl RebuildFlight {
    /// Run `rebuild` under `lock` unless a rebuild that read the op set
    /// after this trigger has already finished. `rebuild` returns `None`
    /// on failure, which covers nothing, so the next trigger rebuilds.
    pub async fn run<T, F, Fut>(&self, lock: &tokio::sync::Mutex<()>, rebuild: F) -> Option<T>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Option<T>>,
    {
        let ticket = self.requested.fetch_add(1, Ordering::SeqCst) + 1;
        let _installing = lock.lock().await;
        if self.built.load(Ordering::SeqCst) >= ticket {
            return None;
        }
        // Every ticket issued so far was issued before the op-set read
        // `rebuild` makes, so its result covers them all.
        let covers = self.requested.load(Ordering::SeqCst);
        let built = rebuild().await?;
        self.built.fetch_max(covers, Ordering::SeqCst);
        Some(built)
    }
}

#[cfg(test)]
#[path = "rebuild_flight_tests.rs"]
mod tests;
