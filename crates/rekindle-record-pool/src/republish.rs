//! Own-write republish (plan E3.4 LV1, lead decision OW1).
//!
//! A write to one of our own records can come back stored while it reached
//! only one side of a split network: Veilid 0.5.8 counts a set as landed
//! once `set_value_count` (5) of the nodes its fanout reached accept it,
//! closest or not (`set_value.rs:441`), and never re-pushes it; a get after
//! the heal settles on `get_value_count` (3) holders of the newest seq it
//! saw (`get_value.rs:328`, `:498-512`), so readers can settle on the old
//! value. Veilid's own rehydration skips records this node created this
//! session (`open_record.rs:62-81`) and runs only on a `Hold` re-open,
//! which stops when we leave a community (plan, leave3).
//!
//! So after a write that stored (`Landed` or `Unchanged`) through
//! [`RecordPool::set_durable`] or [`RecordPool::set_republished`], the same
//! bytes are re-sent at the same seq: a set whose data and writer equal the
//! local copy keeps its seq and resends the stored bytes
//! (`set_value.rs:353-366`), which a store already holding it takes as
//! nothing new. Re-sends come 60 s after the write, then at doubling waits
//! up to 30 min, until 24 h have passed or a newer value replaced it
//! (a newer write through either call, any other write that changed our
//! local copy, or a newer value the network holds). One entry per (record,
//! subkey); the newest write replaces the older one's entry.
//!
//! Session-scoped: the entries live in the session's pool, not in a
//! community's scope, so a leave's last write is re-sent after the leave
//! shut its community down. Logout's drain stops the task; nothing is
//! persisted. The writer is kept in memory only, like a held write's.
//!
//! Present-tense writes (presence beats, the profile status, registry
//! rows) use plain [`RecordPool::set`] and are never re-sent: their writer
//! rewrites them every beat, and a stale one re-sent would claim a
//! presence we no longer have.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use rekindle_records::lease::LeaseId;
use rekindle_records::port::{OpenIntent, PortError, SetOutcome, WriterKey};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::{set_failed, RecordPool};

/// The first re-send's wait after the write.
const FIRST: Duration = Duration::from_secs(60);
/// The longest wait between two re-sends.
const CAP: Duration = Duration::from_secs(30 * 60);
/// How long after the write re-sends go on.
const WINDOW: Duration = Duration::from_secs(24 * 60 * 60);

/// The wait before re-send `sent + 1`, after `sent` re-sends: 60 s,
/// doubling, capped at 30 min.
fn wait_after(sent: u32) -> Duration {
    FIRST.saturating_mul(1 << sent.min(5)).min(CAP)
}

/// When re-send `sent + 1` falls due, the last one having gone at `last`;
/// `None` once that is past the window that opened at `since`.
fn next_due(since: Instant, sent: u32, last: Instant) -> Option<Instant> {
    let due = last + wait_after(sent);
    (due <= since + WINDOW).then_some(due)
}

/// One own write being re-sent.
struct Entry {
    data: Vec<u8>,
    writer: WriterKey,
    since: Instant,
    sent: u32,
    due: Instant,
}

/// The pool's own-write republish state.
#[derive(Default)]
pub(super) struct Republish {
    entries: Mutex<BTreeMap<(String, u32), Entry>>,
    changed: tokio::sync::Notify,
    started: AtomicBool,
}

/// What one re-send found.
#[derive(Debug, Clone, Copy)]
enum Resent {
    /// Sent at the same seq; the network holds ours.
    Sent,
    /// Not sent, or the network holds a newer value: a newer write won.
    Superseded,
    /// Not stored this time (offline, unreachable): the next one tries.
    Missed,
    /// Veilid refused the write itself: no re-send can land it.
    Refused,
}

impl RecordPool {
    /// [`set`](Self::set) for a value of our own record that every reader
    /// must reach (a member record's card, routes, op-log head and
    /// frontier): once stored, it is re-sent (OW1, this module). A miss is
    /// returned to the caller, which owns its retry.
    ///
    /// # Errors
    /// As [`set`](Self::set).
    pub async fn set_republished(
        &self,
        id: LeaseId,
        subkey: u32,
        data: Vec<u8>,
        writer: Option<WriterKey>,
    ) -> Result<SetOutcome, PortError> {
        let (key, _) = self.lookup(id)?;
        let outcome = self.set(id, subkey, data.clone(), writer.clone()).await?;
        if matches!(outcome, SetOutcome::Landed | SetOutcome::Unchanged) {
            self.republish_after(&key, subkey, data, writer);
        }
        Ok(outcome)
    }

    /// Re-send `data`, which just stored on `key`'s `subkey`, from 60 s on.
    pub(super) fn republish_after(
        &self,
        key: &str,
        subkey: u32,
        data: Vec<u8>,
        writer: Option<WriterKey>,
    ) {
        let Some(writer) = writer.or_else(|| self.table.lock().writer(key).cloned()) else {
            return;
        };
        if self.repush_stop.is_cancelled() {
            return;
        }
        let now = Instant::now();
        self.republish.entries.lock().insert(
            (key.to_owned(), subkey),
            Entry {
                data,
                writer,
                since: now,
                sent: 0,
                due: now + wait_after(0),
            },
        );
        self.start_republish();
        self.republish.changed.notify_one();
    }

    /// Own writes being re-sent, by (record key, subkey).
    #[must_use]
    pub fn republishing(&self) -> Vec<(String, u32)> {
        self.republish.entries.lock().keys().cloned().collect()
    }

    fn start_republish(&self) {
        let Some(pool) = self.this.upgrade() else {
            return;
        };
        if self.republish.started.swap(true, Ordering::AcqRel) {
            return;
        }
        self.scope
            .spawn_with_token_or_drop("own write republish", move |stop| async move {
                pool.republish_loop(stop).await;
            });
    }

    async fn republish_loop(self: Arc<Self>, stop: CancellationToken) {
        loop {
            let next = self.republish.entries.lock().values().map(|e| e.due).min();
            tokio::select! {
                biased;
                () = stop.cancelled() => return,
                () = self.repush_stop.cancelled() => return,
                () = async {
                    match next {
                        Some(due) => tokio::time::sleep_until(due).await,
                        None => std::future::pending().await,
                    }
                } => {}
                () = self.republish.changed.notified() => continue,
            }
            self.republish_due().await;
        }
    }

    /// Re-send every entry that is due, one at a time.
    async fn republish_due(&self) {
        let now = Instant::now();
        let due: Vec<((String, u32), Vec<u8>, WriterKey)> = self
            .republish
            .entries
            .lock()
            .iter()
            .filter(|(_, e)| e.due <= now)
            .map(|(slot, e)| (slot.clone(), e.data.clone(), e.writer.clone()))
            .collect();
        for (slot, data, writer) in due {
            if self.repush_stop.is_cancelled() {
                return;
            }
            let resent = self.resend(&slot, data.clone(), writer).await;
            let mut entries = self.republish.entries.lock();
            // A newer write replaced the entry meanwhile: it starts its own
            // schedule.
            let Some(entry) = entries.get_mut(&slot) else {
                continue;
            };
            if entry.data != data {
                continue;
            }
            let attempt = entry.sent.saturating_add(1);
            let next = match resent {
                Resent::Sent | Resent::Missed => next_due(entry.since, attempt, Instant::now()),
                Resent::Superseded | Resent::Refused => None,
            };
            match next {
                Some(due) => {
                    entry.sent = attempt;
                    entry.due = due;
                }
                None => {
                    entries.remove(&slot);
                }
            }
            tracing::debug!(
                kind = "own record subkey",
                subkey = slot.1,
                attempt,
                outcome = ?resent,
                last = next.is_none(),
                "own write re-sent"
            );
        }
    }

    /// Re-send `data` to `slot` at its seq, if our local copy still holds
    /// it from `writer`.
    async fn resend(&self, slot: &(String, u32), data: Vec<u8>, writer: WriterKey) -> Resent {
        let lease = match self.acquire(&slot.0, None, OpenIntent::Check).await {
            Ok(lease) => lease,
            Err(PortError::Unreachable(_) | PortError::Closed) => return Resent::Missed,
            Err(_) => return Resent::Refused,
        };
        let (key, subkey) = slot.clone();
        let result = self
            .run("dht republish", move |backend| async move {
                // Only the bytes our copy holds at its seq: any other value
                // would be written at the next seq, over a newer write.
                let local = backend.get(&key, subkey, false).await?;
                if !local.is_some_and(|v| v.data == data && v.writer == writer.public) {
                    return Ok(Resent::Superseded);
                }
                let settled = backend
                    .set(&key, subkey, data.clone(), Some(&writer))
                    .await?;
                Ok(match settled {
                    Some(v) if v.data != data || v.writer != writer.public => Resent::Superseded,
                    _ => Resent::Sent,
                })
            })
            .await;
        self.release(lease).await;
        match result {
            Ok(resent) => resent,
            Err(e) => match set_failed(&e) {
                Ok(_) | Err(PortError::Unreachable(_) | PortError::Closed) => Resent::Missed,
                Err(_) => Resent::Refused,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn re_sends_wait_a_minute_then_double_to_half_an_hour() {
        let waits: Vec<u64> = (0..8).map(|n| wait_after(n).as_secs()).collect();
        assert_eq!(waits, [60, 120, 240, 480, 960, 1800, 1800, 1800]);
        assert_eq!(wait_after(u32::MAX), CAP);
    }

    #[test]
    fn re_sends_stop_once_a_day_has_passed() {
        let since = Instant::now();
        let (mut last, mut sent, mut times) = (since, 0, 0u32);
        while let Some(due) = next_due(since, sent, last) {
            assert!(due <= since + WINDOW);
            last = due;
            sent += 1;
            times += 1;
        }
        // 60+120+…+960 = 1860 s, then 30-min waits to the 24 h mark:
        // 5 + (86_400 − 1_860) / 1_800 = 5 + 46.
        assert_eq!(times, 51);
        assert!(next_due(since, sent, last).is_none());
        assert!(last + CAP > since + WINDOW);
    }
}
