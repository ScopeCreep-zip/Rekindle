//! Watches, and the one owner of watch death.

use std::sync::Arc;
use std::time::{Duration, Instant};

use rekindle_records::lease::{LeaseId, SubkeySet, WatchPlan};
use veilid_core::{RecordKey, ValueData, VeilidValueChange};

use super::{range, to_protocol, RecordPool};
use crate::ProtocolError;

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

/// What a `ValueChange` update meant, for the caller that dispatches it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueChangeReport {
    /// The pool holds the record. When it does, a dead watch is the pool's
    /// to re-arm; nothing else re-watches it.
    pub held: bool,
    /// The watch ended (`count == 0`) or died (no subkeys). Held and still
    /// wanted, it is being re-armed.
    pub watch_died: bool,
    /// Subkeys that changed. A watch's last notification can carry changes
    /// too, so these are reported even when `watch_died` is set.
    pub subkeys: Vec<u32>,
    /// The first changed subkey's value when Veilid sent one; otherwise the
    /// caller force-refreshes each subkey (R12).
    pub value: Option<ValueData>,
}

impl RecordPool {
    /// Set the subkeys this borrower watches (empty stops its watch).
    ///
    /// # Errors
    /// The lease is not held, or Veilid refused the watch request.
    pub async fn watch(&self, id: LeaseId, subkeys: SubkeySet) -> Result<(), ProtocolError> {
        let (key, _) = self.lookup(id)?;
        let plan = self.table.lock().set_watch(id, subkeys);
        self.apply_watch(&key, plan).await
    }

    /// Watch every subkey of the leased record (its schema's subkey count).
    ///
    /// # Errors
    /// As [`watch`](Self::watch).
    pub async fn watch_all(&self, id: LeaseId) -> Result<(), ProtocolError> {
        let (_, opened) = self.lookup(id)?;
        self.watch(id, (0..opened.subkey_count).collect()).await
    }

    pub(super) async fn apply_watch(
        &self,
        key: &RecordKey,
        plan: WatchPlan,
    ) -> Result<(), ProtocolError> {
        let k = key.clone();
        match plan {
            WatchPlan::Unchanged => Ok(()),
            WatchPlan::Watch(subkeys) => {
                let set = range(&subkeys);
                // The bool records only that the desired watch state was
                // accepted locally, never that a node granted it (V20), so it
                // is not read: a refused watch shows up as a dead-watch update.
                self.run("dht watch", move |rc| async move {
                    rc.watch_dht_values(k, Some(set), None, None).await
                })
                .await
                .map(drop)
                .map_err(|e| to_protocol(&e))
            }
            WatchPlan::Cancel => self
                .run("dht cancel watch", move |rc| async move {
                    rc.cancel_dht_watch(k, None).await
                })
                .await
                .map(drop)
                .map_err(|e| to_protocol(&e)),
        }
    }

    /// Classify a `ValueChange` update and, for a dead watch on a record a
    /// lease still watches, re-arm it with backoff (the only re-arm path).
    pub fn on_value_change(self: &Arc<Self>, change: &VeilidValueChange) -> ValueChangeReport {
        let name = change.key.to_string();
        let (held, watched) = {
            let table = self.table.lock();
            (table.is_held(&name), table.watched(&name))
        };
        let report = ValueChangeReport {
            held,
            watch_died: change.count == 0 || change.subkeys.is_empty(),
            subkeys: change.subkeys.iter().collect(),
            value: change.value.clone(),
        };
        if !report.held || !report.watch_died || watched.is_empty() {
            return report;
        }
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
        let key = change.key.clone();
        self.scope
            .spawn_with_token_or_drop("dht watch re-arm", move |stop| async move {
                if stop
                    .run_until_cancelled(tokio::time::sleep(delay))
                    .await
                    .is_none()
                {
                    return;
                }
                // Still wanted? The lease may have gone during the backoff.
                let subkeys = pool.table.lock().watched(&key.to_string());
                if subkeys.is_empty() {
                    return;
                }
                if let Err(e) = pool.apply_watch(&key, WatchPlan::Watch(subkeys)).await {
                    tracing::warn!(record = %key, error = %e, "re-arming DHT watch failed");
                }
            });
        report
    }
}
