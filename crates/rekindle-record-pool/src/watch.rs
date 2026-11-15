//! Watches, and the one owner of watch death.

use std::sync::Arc;
use std::time::Duration;

use rekindle_records::lease::{LeaseId, SubkeySet, WatchPlan};
use tokio::time::Instant;

use super::{range, to_port, RecordPool};
use rekindle_records::port::PortError;

/// First and longest delay before re-arming a dead watch.
const REARM_MIN: Duration = Duration::from_secs(1);
const REARM_MAX: Duration = Duration::from_secs(60);
/// A watch that stayed up this long has its backoff forgiven.
const REARM_FORGIVE: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, Copy)]
pub(super) struct Rearm {
    deaths: u32,
    last: Instant,
}

use rekindle_records::port::{ValueChange, ValueChangeReport};

impl RecordPool {
    /// Set the subkeys this borrower watches (empty stops its watch).
    ///
    /// # Errors
    /// The lease is not held, or Veilid refused the watch request.
    pub async fn watch(&self, id: LeaseId, subkeys: SubkeySet) -> Result<(), PortError> {
        let (key, _) = self.lookup(id)?;
        let plan = self.table.lock().set_watch(id, subkeys);
        self.apply_watch(&key, plan).await
    }

    /// Watch every subkey of the leased record (its schema's subkey count).
    ///
    /// # Errors
    /// As [`watch`](Self::watch).
    pub async fn watch_all(&self, id: LeaseId) -> Result<(), PortError> {
        let (_, opened) = self.lookup(id)?;
        self.watch(id, (0..opened.subkey_count).collect()).await
    }

    pub(super) async fn apply_watch(&self, key: &str, plan: WatchPlan) -> Result<(), PortError> {
        let k = key.to_string();
        match plan {
            WatchPlan::Unchanged => Ok(()),
            WatchPlan::Watch(subkeys) => {
                let set = range(&subkeys);
                // The bool records only that the desired watch state was
                // accepted locally, never that a node granted it (V20), so it
                // is not read: a refused watch shows up as a dead-watch update.
                self.run("dht watch", move |backend| async move {
                    backend.watch(&k, Some(&set), None, None).await
                })
                .await
                .map(drop)
                .map_err(|e| to_port(&e))
            }
            WatchPlan::Cancel => self
                .run("dht cancel watch", move |backend| async move {
                    backend.cancel_watch(&k, None).await
                })
                .await
                .map(drop)
                .map_err(|e| to_port(&e)),
        }
    }

    /// Classify a `ValueChange` update and, for a dead watch on a record a
    /// lease still watches, re-arm it with backoff (the only re-arm path).
    pub fn on_value_change(self: &Arc<Self>, change: &ValueChange) -> ValueChangeReport {
        let name = change.key.clone();
        let (held, watched) = {
            let table = self.table.lock();
            (table.is_held(&name), table.watched(&name))
        };
        let report = ValueChangeReport {
            held,
            watch_died: change.count == 0 || change.subkeys.is_empty(),
            subkeys: change.subkeys.clone(),
            value: change.value.clone(),
        };
        if !report.held || !report.watch_died || watched.is_empty() {
            return report;
        }
        // Checked before the re-arm is counted or logged; the key came from
        // the backend's own text, so this holds.
        let Ok(key) = self.backend.check_key(&change.key) else {
            return report;
        };
        let delay = {
            let now = Instant::now();
            let mut rearm = self.rearm.lock();
            let state = rearm.entry(name.clone()).or_insert(Rearm {
                deaths: 0,
                last: now,
            });
            if now.duration_since(state.last) > REARM_FORGIVE {
                state.deaths = 0;
            }
            state.deaths = state.deaths.saturating_add(1);
            state.last = now;
            REARM_MIN
                .saturating_mul(1 << state.deaths.saturating_sub(1).min(6))
                .min(REARM_MAX)
        };
        tracing::info!(record = %name, delay_ms = delay.as_millis(), "DHT watch died; re-arming");
        let pool = Arc::clone(self);
        self.scope
            .spawn_with_token_or_drop("dht watch re-arm", move |stop| async move {
                // A drain (logout) stops it as it stops the re-push: the
                // ending session re-arms nothing (live R1-2).
                tokio::select! {
                    () = stop.cancelled() => return,
                    () = pool.repush_stop.cancelled() => return,
                    () = tokio::time::sleep(delay) => {}
                }
                // Still wanted? The lease may have gone during the backoff.
                let subkeys = pool.table.lock().watched(&key);
                if subkeys.is_empty() {
                    return;
                }
                match pool.apply_watch(&key, WatchPlan::Watch(subkeys)).await {
                    Ok(()) => {}
                    // Drained while the re-arm ran: the session ended.
                    Err(PortError::Closed) => {
                        tracing::debug!(record = %key, "session ended during a watch re-arm");
                    }
                    Err(e) => {
                        tracing::warn!(record = %key, error = %e, "re-arming DHT watch failed");
                    }
                }
            });
        report
    }
}
