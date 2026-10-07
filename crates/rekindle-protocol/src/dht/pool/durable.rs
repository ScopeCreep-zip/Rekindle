//! The one owner of missed own-record writes (plan C7, the C7.4-run item).
//!
//! With `allow_offline: false` Veilid stores and queues nothing for a write
//! that misses consensus (V20), so before this a missed profile field or
//! route blob stayed missing until the next login. A **durable** write
//! ([`RecordPool::set_durable`]) that misses is held here as the latest
//! wanted value for its (record, subkey) and re-pushed with backoff, and at
//! once when the network turns ready, until it lands, is `Unchanged`, or a
//! newer value is found on the network (`Superseded`, which the next
//! durable write settles). A newer durable write replaces a held one. This
//! is Signal's job queue and Matrix's unsent-event queue: retry until
//! acknowledged. It is not a second call-retry layer: `run_retrying` covers
//! transient errors within one call, this carries a write across calls.
//!
//! Present-tense writes (presence heartbeats, slot claims) use plain
//! [`RecordPool::set`] and are never held: a heartbeat landing late would
//! tell readers we were reachable when we were not.
//!
//! Held writes outlive the session (plan C7.6h): logout exports them
//! ([`RecordPool::take_held`]) for the host to persist, and the next login
//! of the same identity restores them ([`RecordPool::restore_held`]). The
//! writer is always the record's own lease (every durable write targets a
//! record the session holds writable), so no key is held or persisted.
//!
//! A held write is re-pushed only while it can be written: it carries its own
//! writer ([`RecordPool::set_durable_as`]: a member slot of a record whose
//! sticky writer is another key), or the session holds its record writable.
//! Opened without its writer the record cannot take the write (Veilid:
//! "value is not writable"). A held write's writer is kept in memory only,
//! beside the lease table's, and never persisted: a restored write has none
//! and waits until the session's own open of its record (`acquire` with the
//! writer), which makes it due at once ([`RecordPool::wake_held`]).
//!
//! Every settle is published ([`RecordPool::settled`]): a write that landed,
//! or one the network superseded with a newer value. A writer whose page
//! merges (a channel slot) learns there that its held value was replaced
//! and its entries did not land (plan C7.13).

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use rekindle_records::lease::LeaseId;
use tokio::time::Instant;
use veilid_core::{KeyPair, RecordKey};

use super::io::SetOutcome;
use super::RecordPool;
use crate::ProtocolError;

/// First and longest wait before re-pushing a held write.
const REPUSH_MIN: Duration = Duration::from_secs(2);
const REPUSH_MAX: Duration = Duration::from_secs(60);

/// A durable write that has not landed yet.
#[derive(Debug)]
pub(super) struct HeldWrite {
    key: RecordKey,
    data: Vec<u8>,
    /// The write's own writer, when not the record's sticky one. In memory
    /// only.
    writer: Option<KeyPair>,
    attempts: u32,
    due: Instant,
}

/// One due re-push, copied out of the held map.
struct DuePush {
    slot: (String, u32),
    key: RecordKey,
    data: Vec<u8>,
    writer: Option<KeyPair>,
}

/// A durable write that settled: it landed (or the network already held
/// it), or the network held a newer value and this one was dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settled {
    pub record_key: String,
    pub subkey: u32,
    pub landed: bool,
}

/// A held write handed to the host at logout, or back to the next session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsentWrite {
    pub record_key: String,
    pub subkey: u32,
    pub data: Vec<u8>,
}

/// Held writes by (record key, subkey).
pub(super) type HeldWrites = HashMap<(String, u32), HeldWrite>;

fn backoff(attempts: u32) -> Duration {
    REPUSH_MIN
        .saturating_mul(1 << attempts.min(5))
        .min(REPUSH_MAX)
}

impl RecordPool {
    /// [`set`](Self::set) for state that must reach the network eventually
    /// (profile fields, route blobs, the friend list, the account header).
    /// A miss is held and re-pushed until it lands; a newer durable write
    /// to the same subkey replaces it. The outcome returned is this
    /// attempt's.
    ///
    /// # Errors
    /// As [`set`](Self::set).
    pub async fn set_durable(
        &self,
        id: LeaseId,
        subkey: u32,
        data: Vec<u8>,
    ) -> Result<SetOutcome, ProtocolError> {
        self.set_durable_as(id, subkey, data, None).await
    }

    /// [`set_durable`](Self::set_durable) as `writer` when it differs from
    /// the lease's sticky writer (a member slot of a record we created under
    /// its owner key). A held write keeps the writer in memory for its
    /// re-pushes.
    ///
    /// # Errors
    /// As [`set`](Self::set).
    pub async fn set_durable_as(
        &self,
        id: LeaseId,
        subkey: u32,
        data: Vec<u8>,
        writer: Option<KeyPair>,
    ) -> Result<SetOutcome, ProtocolError> {
        let (key, _) = self.lookup(id)?;
        let slot = (key.to_string(), subkey);
        let outcome = match self.set(id, subkey, data.clone(), writer.clone()).await {
            Ok(outcome) => outcome,
            // Logout released the wait: the write may or may not land, so
            // it is held like a miss (and persisted at logout, C7.6h).
            Err(ProtocolError::PoolClosed) => {
                self.hold(slot, key, data, writer);
                return Err(ProtocolError::PoolClosed);
            }
            Err(e) => return Err(e),
        };
        match outcome {
            SetOutcome::BelowConsensus | SetOutcome::Offline => {
                self.hold(slot, key, data, writer);
            }
            // Returned to the caller, whose next write settles the slot
            // (a merging writer's compare-and-swap); published only from a
            // re-push, which has no caller.
            SetOutcome::Superseded(_) => {
                self.held.lock().remove(&slot);
            }
            SetOutcome::Landed | SetOutcome::Unchanged => {
                self.held.lock().remove(&slot);
                self.publish_settled(&slot, &outcome);
            }
        }
        Ok(outcome)
    }

    /// Writes that settled from now on (plan C7.13). A receiver that lags
    /// past the buffer misses the oldest; its pending entries stay pending
    /// until the slot's next settle.
    #[must_use]
    pub fn settled(&self) -> tokio::sync::broadcast::Receiver<Settled> {
        self.settled.subscribe()
    }

    fn publish_settled(&self, slot: &(String, u32), outcome: &SetOutcome) {
        let _ = self.settled.send(Settled {
            record_key: slot.0.clone(),
            subkey: slot.1,
            landed: !matches!(outcome, SetOutcome::Superseded(_)),
        });
    }

    /// Hold a write as the latest wanted value for its slot, and wake the
    /// re-push task (started on first use; never after drain).
    fn hold(&self, slot: (String, u32), key: RecordKey, data: Vec<u8>, writer: Option<KeyPair>) {
        self.held.lock().insert(
            slot,
            HeldWrite {
                key,
                data,
                writer,
                attempts: 0,
                due: Instant::now() + REPUSH_MIN,
            },
        );
        self.start_repush();
        self.held_changed.notify_one();
    }

    /// Writes held for re-push, by (record key, subkey).
    #[must_use]
    pub fn held_writes(&self) -> Vec<(String, u32)> {
        self.held.lock().keys().cloned().collect()
    }

    /// Hand every held write to the host and forget it (logout, after
    /// [`drain`](Self::drain) stopped the re-push).
    #[must_use]
    pub fn take_held(&self) -> Vec<UnsentWrite> {
        self.held
            .lock()
            .drain()
            .map(|((record_key, subkey), write)| UnsentWrite {
                record_key,
                subkey,
                data: write.data,
            })
            .collect()
    }

    /// Hold the writes a previous logout of this identity left unsent, due
    /// after the first backoff step (at once when the network turns ready).
    /// Called before the session writes anything, so its
    /// own newer durable writes replace them slot by slot. A key that does
    /// not parse is logged and dropped.
    pub fn restore_held(&self, writes: Vec<UnsentWrite>) {
        for write in writes {
            let Ok(key) = write.record_key.parse::<RecordKey>() else {
                tracing::warn!(record = %write.record_key, "dropping an unsent write with a bad key");
                continue;
            };
            self.hold((write.record_key, write.subkey), key, write.data, None);
        }
    }

    /// Whether the session holds `record` writable (its lease has a writer).
    fn writable(&self, record: &str) -> bool {
        self.table.lock().writer(record).is_some()
    }

    /// The session just opened `record` with its writer: its held writes are
    /// due now.
    pub(super) fn wake_held(&self, record: &str) {
        let now = Instant::now();
        let mut woke = false;
        for ((key, _), write) in self.held.lock().iter_mut() {
            if key == record {
                write.due = now;
                woke = true;
            }
        }
        if woke {
            self.held_changed.notify_one();
        }
    }

    /// Start the re-push task on the pool's scope, once.
    fn start_repush(&self) {
        let Some(pool) = self.this.upgrade() else {
            return;
        };
        if self.repush_stop.is_cancelled() || self.repush_started.swap(true, Ordering::AcqRel) {
            return;
        }
        self.scope
            .spawn_with_token_or_drop("dht write re-push", move |stop| async move {
                pool.repush_loop(stop).await;
            });
    }

    async fn repush_loop(self: Arc<Self>, stop: tokio_util::sync::CancellationToken) {
        let mut ready = self.ready.clone();
        loop {
            // Only writable records' writes can be due; the others wait for
            // `wake_held`.
            let held: Vec<(String, bool, Instant)> = self
                .held
                .lock()
                .iter()
                .map(|((key, _), write)| (key.clone(), write.writer.is_some(), write.due))
                .collect();
            let next_due = held
                .into_iter()
                .filter(|(key, own_writer, _)| *own_writer || self.writable(key))
                .map(|(_, _, due)| due)
                .min();
            tokio::select! {
                () = stop.cancelled() => return,
                () = self.repush_stop.cancelled() => return,
                () = self.held_changed.notified() => continue,
                changed = ready.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    if *ready.borrow() {
                        // Back online: everything held is due now.
                        let now = Instant::now();
                        for write in self.held.lock().values_mut() {
                            write.due = now;
                        }
                    }
                    continue;
                }
                () = async {
                    match next_due {
                        Some(due) => tokio::time::sleep_until(due).await,
                        None => std::future::pending().await,
                    }
                } => {}
            }
            if self.repush_stop.is_cancelled() {
                return;
            }
            self.repush_due().await;
        }
    }

    /// Re-push every held write that is due, one at a time.
    async fn repush_due(&self) {
        let now = Instant::now();
        let due: Vec<DuePush> = self
            .held
            .lock()
            .iter()
            .filter(|(_, w)| w.due <= now)
            .map(|(slot, w)| DuePush {
                slot: slot.clone(),
                key: w.key.clone(),
                data: w.data.clone(),
                writer: w.writer.clone(),
            })
            .collect();
        for DuePush {
            slot,
            key,
            data,
            writer,
        } in due
        {
            // Drain stops the re-push before its next write; what is left
            // stays held.
            if self.repush_stop.is_cancelled() {
                return;
            }
            // No writer of its own and not held writable (yet):
            // `wake_held` re-schedules it.
            if writer.is_none() && !self.writable(&slot.0) {
                continue;
            }
            let pushed = match self.acquire(&key, None).await {
                Ok(lease) => {
                    let outcome = self.set(lease, slot.1, data.clone(), writer).await;
                    self.release(lease).await;
                    outcome
                }
                Err(e) => Err(e),
            };
            let mut held = self.held.lock();
            // A newer durable write replaced this one meanwhile: it is the
            // one to keep, whatever happened to the older value.
            let Some(entry) = held.get_mut(&slot) else {
                continue;
            };
            if entry.data != data {
                continue;
            }
            match pushed {
                // A miss, an unreachable record or a logout: try again later.
                Ok(SetOutcome::BelowConsensus | SetOutcome::Offline)
                | Err(ProtocolError::DhtRecordUnreachable(_) | ProtocolError::PoolClosed) => {
                    entry.attempts = entry.attempts.saturating_add(1);
                    entry.due = Instant::now() + backoff(entry.attempts);
                }
                // Veilid refused the write itself (schema, size, writer): no
                // re-push can land it. Settle it as not landed, so its
                // writer learns (a channel's pending messages fail visibly).
                Err(error) => {
                    tracing::warn!(record = %slot.0, subkey = slot.1, %error, "held write refused; dropping it");
                    held.remove(&slot);
                    let _ = self.settled.send(Settled {
                        record_key: slot.0.clone(),
                        subkey: slot.1,
                        landed: false,
                    });
                }
                Ok(outcome) => {
                    tracing::debug!(record = %slot.0, subkey = slot.1, ?outcome, "held write settled");
                    held.remove(&slot);
                    self.publish_settled(&slot, &outcome);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_from_two_seconds_to_a_minute() {
        assert_eq!(backoff(0), Duration::from_secs(2));
        assert_eq!(backoff(1), Duration::from_secs(4));
        assert_eq!(backoff(4), Duration::from_secs(32));
        assert_eq!(backoff(5), Duration::from_secs(60));
        assert_eq!(backoff(30), Duration::from_secs(60));
    }
}
