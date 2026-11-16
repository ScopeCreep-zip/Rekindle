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
mod tests {
    use super::RebuildFlight;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// K triggers while one rebuild runs give that rebuild and one more.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn triggers_during_a_rebuild_coalesce_into_one_more() {
        let flight = Arc::new(RebuildFlight::default());
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        let runs = Arc::new(AtomicUsize::new(0));
        let (started_tx, started_rx) = tokio::sync::oneshot::channel::<()>();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
        let first = {
            let (flight, lock, runs) = (flight.clone(), lock.clone(), runs.clone());
            tokio::spawn(async move {
                flight
                    .run(&lock, || async move {
                        runs.fetch_add(1, Ordering::SeqCst);
                        started_tx.send(()).ok();
                        release_rx.await.ok();
                        Some(())
                    })
                    .await
            })
        };
        started_rx.await.expect("the first rebuild starts");
        let mut waiting = Vec::new();
        for _ in 0..8 {
            let (flight, lock, runs) = (flight.clone(), lock.clone(), runs.clone());
            waiting.push(tokio::spawn(async move {
                flight
                    .run(&lock, || async move {
                        runs.fetch_add(1, Ordering::SeqCst);
                        tokio::task::yield_now().await;
                        Some(())
                    })
                    .await
            }));
        }
        // Every waiter has taken its ticket before the first finishes.
        while flight.requested.load(Ordering::SeqCst) < 9 {
            tokio::task::yield_now().await;
        }
        release_tx.send(()).expect("the first rebuild waits");
        first.await.expect("first");
        for task in waiting {
            task.await.expect("waiter");
        }
        assert_eq!(runs.load(Ordering::SeqCst), 2);
    }

    /// A failed rebuild covers nothing: the next trigger rebuilds.
    #[tokio::test]
    async fn a_failed_rebuild_covers_nothing() {
        let flight = RebuildFlight::default();
        let lock = tokio::sync::Mutex::new(());
        assert_eq!(flight.run(&lock, || async { None::<()> }).await, None);
        assert_eq!(flight.run(&lock, || async { Some(1) }).await, Some(1));
    }
}
