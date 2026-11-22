//! Watches, and the one owner of watch death.

use std::sync::Arc;
use std::time::Duration;

use rekindle_records::lease::{LeaseId, SubkeySet, WatchPlan};
use tokio::time::Instant;

use super::{range, to_port, RecordPool};
use rekindle_records::port::PortError;

/// Whether an open record can still change (plan KA1). The Host marks a
/// record [`Immutable`](Self::Immutable) where it opens one (ADR 0011: the
/// anchor after its GenesisCert, a sealed DFLT(1) page); the keepalive
/// re-opens it (`Hold`) and never inspects or fetches it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mutability {
    Mutable,
    Immutable,
}

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
        let plan = {
            let mut table = self.table.lock();
            match table.set_watch(id, subkeys) {
                // The table already wants this watch, but Veilid never
                // accepted it: apply it again rather than call it set.
                WatchPlan::Unchanged if self.unarmed.lock().contains(&key) => {
                    let wanted = table.watched(&key);
                    if wanted.is_empty() {
                        WatchPlan::Unchanged
                    } else {
                        WatchPlan::Watch(wanted)
                    }
                }
                plan => plan,
            }
        };
        self.arm(&key, plan).await
    }

    /// Apply `plan` and record whether Veilid holds the watch: a record is
    /// armed only once `watch_dht_values` accepted it, and one it refused is
    /// re-armed with the dead-watch backoff, so a watch that never armed is
    /// retried here, the one retry layer (D4).
    async fn arm(&self, key: &str, plan: WatchPlan) -> Result<(), PortError> {
        let watching = matches!(plan, WatchPlan::Watch(_));
        let applied = self.apply_watch(key, plan).await;
        match (&applied, watching) {
            (Ok(()), _) => {
                self.unarmed.lock().remove(key);
            }
            (Err(_), true) => {
                self.unarmed.lock().insert(key.to_string());
                self.schedule_rearm(key);
            }
            (Err(_), false) => {}
        }
        applied
    }

    /// Watch every subkey of the leased record (its schema's subkey count).
    ///
    /// # Errors
    /// As [`watch`](Self::watch).
    pub async fn watch_all(&self, id: LeaseId) -> Result<(), PortError> {
        let (_, opened) = self.lookup(id)?;
        self.watch(id, (0..opened.subkey_count).collect()).await
    }

    /// Mark the leased record as one that cannot change (plan KA1, ADR
    /// 0011: the community anchor after its GenesisCert, a sealed DFLT(1)
    /// page). The mark lasts while the record is open in this pool.
    ///
    /// # Errors
    /// The lease is not held.
    pub fn mark_immutable(&self, id: LeaseId) -> Result<(), PortError> {
        let table = self.table.lock();
        let key = table.key(id).ok_or(PortError::LeaseNotHeld(id.0))?;
        self.records
            .lock()
            .get_mut(key)
            .ok_or(PortError::LeaseNotHeld(id.0))?
            .mutability = Mutability::Immutable;
        Ok(())
    }

    /// The leased record's subkeys no armed watch of this node covers: all
    /// of them while the record is unwatched or Veilid has not accepted its
    /// watch, none when an armed watch covers every subkey. An armed watch
    /// delivers each change to its subkeys (0.5.8 `storage_manager/watch_value.rs:2080`
    /// `inbound_value_changed`), so nothing app-side inspects those (D4,
    /// plan KA1 amending VU decision 7). An immutable record has none.
    ///
    /// # Errors
    /// The lease is not held.
    pub(crate) fn unwatched_subkeys(&self, id: LeaseId) -> Result<Vec<u32>, PortError> {
        let (key, opened) = self.lookup(id)?;
        if opened.mutability == Mutability::Immutable {
            return Ok(Vec::new());
        }
        let watched = if self.unarmed.lock().contains(&key) {
            SubkeySet::default()
        } else {
            self.table.lock().watched(&key)
        };
        Ok((0..opened.subkey_count)
            .filter(|subkey| !watched.contains(subkey))
            .collect())
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
        self.note_report(change);
        if !report.held || !report.watch_died || watched.is_empty() {
            return report;
        }
        self.unarmed.lock().insert(name.clone());
        tracing::info!(record = %name, "DHT watch died; re-arming");
        self.schedule_rearm(&name);
        report
    }

    /// Re-arm `name`'s watch after the backoff its deaths and refusals
    /// earned, while a lease still watches it; a refused re-arm schedules
    /// the next one.
    fn schedule_rearm(&self, name: &str) {
        // Checked before the re-arm is counted; the key came from the
        // backend's own text, so this holds.
        let Ok(key) = self.backend.check_key(name) else {
            return;
        };
        let delay = {
            let now = Instant::now();
            let mut rearm = self.rearm.lock();
            let state = rearm.entry(name.to_string()).or_insert(Rearm {
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
        tracing::debug!(record = %name, delay_ms = delay.as_millis(), "DHT watch re-arm scheduled");
        let Some(pool) = self.this.upgrade() else {
            return;
        };
        self.scope
            .spawn_with_token_or_drop("dht watch re-arm", move |stop| async move {
                // A drain (logout) stops it as it stops the re-push: the
                // ending session re-arms nothing (live R1-2).
                // Biased, the stops first: a re-arm that falls due at the
                // instant of a stop or a drain arms nothing.
                tokio::select! {
                    biased;
                    () = stop.cancelled() => return,
                    () = pool.repush_stop.cancelled() => return,
                    () = tokio::time::sleep(delay) => {}
                }
                // Still wanted and still unarmed? The lease may have gone,
                // or an earlier re-arm or a new `watch` armed it.
                let subkeys = pool.table.lock().watched(&key);
                if subkeys.is_empty() || !pool.unarmed.lock().contains(&key) {
                    return;
                }
                match pool.apply_watch(&key, WatchPlan::Watch(subkeys)).await {
                    Ok(()) => {
                        pool.unarmed.lock().remove(&key);
                    }
                    // Drained while the re-arm ran: the session ended.
                    Err(PortError::Closed) => {
                        tracing::debug!(record = %key, "session ended during a watch re-arm");
                    }
                    Err(e) => {
                        tracing::warn!(record = %key, error = %e, "re-arming DHT watch failed");
                        pool.schedule_rearm(&key);
                    }
                }
            });
    }
}
