//! `RecordCloser`: closes an ended session's records off the user's path
//! (plan C7.6i). One per node, living as long as the node.
//!
//! veilid-core's `close_record` takes the record's lifetime lock for
//! writing, which waits for every get, set, inspect and watch in flight on
//! that record through its network fanout (`record_lock_table/mod.rs:21-31`).
//! At logout those calls are still running on the ended pool's scope, and
//! they cannot be cancelled or aborted (an abort mid-commit wedges Veilid's
//! record store). So the closer first waits for the ended pool's calls to
//! finish, then closes the records, in the background. A late re-open of a
//! held record has then finished, so no close is undone by it.
//!
//! Records are always closed: an open record keeps its writer `KeyPair` in
//! Veilid's memory and its watch renewing (`record_store/opened_record.rs`),
//! which must not outlive the identity's session. The next session's pool
//! waits for a record still closing before it opens it again.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use rekindle_lifecycle::SessionScope;
use tokio::sync::watch;
use veilid_core::{RecordKey, RoutingContext};

/// Records being closed, each with the receiver its close completes.
type Closing = Arc<Mutex<HashMap<String, watch::Receiver<bool>>>>;

/// The node's closer of ended sessions' records.
pub struct RecordCloser {
    scope: Arc<SessionScope>,
    closing: Closing,
}

impl std::fmt::Debug for RecordCloser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecordCloser")
            .field("closing", &self.closing.lock().len())
            .finish_non_exhaustive()
    }
}

impl RecordCloser {
    /// A closer whose tasks live as long as the node.
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            scope: SessionScope::new(
                "record close",
                Arc::new(|task| tracing::error!(task, "record close task panicked")),
            ),
            closing: Arc::default(),
        })
    }

    /// Records still being closed.
    #[must_use]
    pub fn closing(&self) -> usize {
        self.closing.lock().len()
    }

    /// Once every call of the ended pool (`calls`) has finished, close
    /// `keys` one by one. Returns at once.
    pub(super) fn close_after(
        &self,
        calls: Arc<SessionScope>,
        rc: RoutingContext,
        keys: Vec<RecordKey>,
    ) {
        let mut done = Vec::with_capacity(keys.len());
        {
            let mut closing = self.closing.lock();
            for key in &keys {
                let (tx, rx) = watch::channel(false);
                closing.insert(key.to_string(), rx);
                done.push(tx);
            }
        }
        let closing = Arc::clone(&self.closing);
        self.scope.spawn_or_drop("record close", async move {
            let started = tokio::time::Instant::now();
            calls.close_and_wait().await;
            let calls_done = started.elapsed();
            let count = keys.len();
            for (key, tx) in keys.into_iter().zip(done) {
                if let Err(e) = rc.close_dht_record(key.clone()).await {
                    tracing::debug!(record = %key, error = %e, "closing record failed");
                }
                closing.lock().remove(&key.to_string());
                let _ = tx.send(true);
            }
            tracing::info!(
                count,
                calls_ms = calls_done.as_millis(),
                close_ms = started.elapsed().saturating_sub(calls_done).as_millis(),
                "ended session's records closed"
            );
        });
    }

    /// Wait while `key` is being closed by an ended session. A close that
    /// never ran (the node is exiting) releases the wait.
    pub(super) async fn wait_closed(&self, key: &str) {
        let Some(mut rx) = self.closing.lock().get(key).cloned() else {
            return;
        };
        tracing::info!(
            record = key,
            "waiting for the previous session to close this record"
        );
        let _ = rx.wait_for(|closed| *closed).await;
    }
}
