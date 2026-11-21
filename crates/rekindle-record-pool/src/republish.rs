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
//! bytes are checked and, where the network lacks them, re-sent at the same seq: a set whose data and writer equal the
//! local copy keeps its seq and resends the stored bytes
//! (`set_value.rs:353-366`), which a store already holding it takes as
//! nothing new. Re-sends come 60 s after the write, then at doubling waits
//! up to 30 min, until 24 h have passed or a newer value replaced it
//! (a newer write through either call, any other write that changed our
//! local copy, or a newer value the network holds). One entry per (record,
//! subkey); the newest write replaces the older one's entry.
//!
//! Each round is inspect-first (lead decision OW2): the due entries are
//! grouped by record and each record gets one set-scope inspect of its due
//! subkeys, the check Veilid's own rehydration makes before it writes
//! (0.5.8 `rehydrate.rs:170-215`, `:371-405`). Only a subkey whose network
//! seq is behind ours, or that the network holds none of, is re-sent; one
//! the network holds at our seq costs nothing more, one it holds newer
//! ends the entry. Veilid's public report carries the seq the set-scope
//! fanout settled on, not its holder count (`dht_record_report.rs:9-20`),
//! so a seq too few hold shows as behind only once `set_value_count`
//! holders of an older one answer first (`inspect_record.rs:672-705`). An
//! inspect that fails sends nothing and leaves every subkey due for the
//! next round. A fully landed record costs one inspect per round.
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

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use rekindle_records::lease::LeaseId;
use rekindle_records::port::{
    OpenIntent, PortError, RecordReport, ReportScope, SetOutcome, WriterKey,
};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::{set_failed, RecordPool};

/// The first re-send's wait after the write.
const FIRST: Duration = Duration::from_secs(60);
/// The longest wait between two re-sends.
const CAP: Duration = Duration::from_secs(30 * 60);
/// How far ahead a due record's other entries join its round.
const JOIN: Duration = Duration::from_secs(60);
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
    /// The network already holds ours at its seq: nothing was sent.
    Held,
    /// Not stored this time (offline, unreachable, or the inspect
    /// failed): the next round tries.
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

    /// One round: the due entries grouped by record, one inspect per
    /// record, and a re-send of only the subkeys the network holds behind
    /// ours (OW2).
    async fn republish_due(&self) {
        let now = Instant::now();
        let by_record = {
            let entries = self.republish.entries.lock();
            let records: BTreeSet<&str> = entries
                .iter()
                .filter(|(_, e)| e.due <= now)
                .map(|((key, _), _)| key.as_str())
                .collect();
            // A due record's other entries falling due within [`JOIN`] join
            // its round: a record written in batches (the prekey record's
            // slots) costs one inspect per round, not one per batch.
            let mut by_record: BTreeMap<String, Vec<Due>> = BTreeMap::new();
            for ((key, subkey), e) in entries.iter() {
                if records.contains(key.as_str()) && e.due <= now + JOIN {
                    by_record.entry(key.clone()).or_default().push(Due {
                        subkey: *subkey,
                        data: e.data.clone(),
                        writer: e.writer.clone(),
                    });
                }
            }
            by_record
        };
        let (mut records, mut resent) = (0usize, 0usize);
        for (key, due) in by_record {
            if self.repush_stop.is_cancelled() {
                return;
            }
            records += 1;
            for (d, found) in self.republish_record(&key, due).await {
                resent += usize::from(d.sent);
                self.settle(&key, &d, found);
            }
        }
        tracing::info!(records, resent, "own write republish round");
    }

    /// Advance or drop `key`'s entry for `d.subkey` after this round found
    /// `found`.
    fn settle(&self, key: &str, d: &Done, found: Resent) {
        let slot = (key.to_owned(), d.subkey);
        let mut entries = self.republish.entries.lock();
        // A newer write replaced the entry meanwhile: it starts its own
        // schedule.
        let Some(entry) = entries.get_mut(&slot) else {
            return;
        };
        if entry.data != d.data {
            return;
        }
        let attempt = entry.sent.saturating_add(1);
        let next = match found {
            Resent::Sent | Resent::Held | Resent::Missed => {
                next_due(entry.since, attempt, Instant::now())
            }
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
            outcome = ?found,
            last = next.is_none(),
            "own write republish checked"
        );
    }

    /// Inspect `key` once for its due subkeys and re-send those the network
    /// holds behind our seq. An inspect that fails sends nothing: every
    /// subkey stays due for the next round.
    async fn republish_record(&self, key: &str, due: Vec<Due>) -> Vec<(Done, Resent)> {
        let lease = match self.acquire(key, None, OpenIntent::Check).await {
            Ok(lease) => Ok(lease),
            Err(PortError::Unreachable(_) | PortError::Closed) => Err(Resent::Missed),
            Err(_) => Err(Resent::Refused),
        };
        let lease = match lease {
            Ok(lease) => lease,
            Err(found) => return due.into_iter().map(|d| (d.done(false), found)).collect(),
        };
        let subkeys: Vec<u32> = due.iter().map(|d| d.subkey).collect();
        let k = key.to_owned();
        // Set scope: the fanout a set uses, counting `set_value_count`
        // holders, starting from nothing local (0.5.8
        // `inspect_record.rs:322-330`, `:426-436`).
        let report = self
            .run("dht republish inspect", move |backend| async move {
                backend
                    .inspect(&k, Some(&subkeys), ReportScope::SyncSet)
                    .await
            })
            .await;
        let mut out = Vec::with_capacity(due.len());
        for d in due {
            let found = match &report {
                Err(_) => Resent::Missed,
                Ok(report) => match standing(report, d.subkey) {
                    Standing::Newer => Resent::Superseded,
                    Standing::Held => Resent::Held,
                    Standing::Behind => {
                        let found = self
                            .resend(key, d.subkey, d.data.clone(), d.writer.clone())
                            .await;
                        out.push((d.done(true), found));
                        continue;
                    }
                },
            };
            out.push((d.done(false), found));
        }
        self.release(lease).await;
        out
    }

    /// Re-send `data` to `key`'s `subkey` at its seq, if our local copy
    /// still holds it from `writer`. The record is leased by the caller.
    async fn resend(&self, key: &str, subkey: u32, data: Vec<u8>, writer: WriterKey) -> Resent {
        let key = key.to_owned();
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
        match result {
            Ok(resent) => resent,
            Err(e) => match set_failed(&e) {
                Ok(_) | Err(PortError::Unreachable(_) | PortError::Closed) => Resent::Missed,
                Err(_) => Resent::Refused,
            },
        }
    }
}

/// One due entry of a round.
struct Due {
    subkey: u32,
    data: Vec<u8>,
    writer: WriterKey,
}

impl Due {
    fn done(self, sent: bool) -> Done {
        Done {
            subkey: self.subkey,
            data: self.data,
            sent,
        }
    }
}

/// A due entry after its round: whether a set went out for it.
struct Done {
    subkey: u32,
    data: Vec<u8>,
    sent: bool,
}

/// Where the network stands on one of our subkeys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Standing {
    /// The network's seq is ours: landed, nothing to send.
    Held,
    /// The network's seq is behind ours, or it holds none.
    Behind,
    /// The network holds a newer seq: a newer write won.
    Newer,
}

/// `subkey`'s standing in an inspect `report`. Veilid reports, per subkey,
/// the seq the set-scope fanout settled on: the first seq
/// `set_value_count` holders agreed on, a newer one seen after that
/// discarded (0.5.8 `inspect_record.rs:672-705`). A subkey the report
/// leaves out is behind (a re-send checks the local copy first).
fn standing(report: &RecordReport, subkey: u32) -> Standing {
    let Some(i) = report.subkeys.iter().position(|s| *s == subkey) else {
        return Standing::Behind;
    };
    let local = report.local_seqs.get(i).copied().flatten();
    let network = report.network_seqs.get(i).copied().flatten();
    match (local, network) {
        (Some(l), Some(n)) if n == l => Standing::Held,
        (Some(l), Some(n)) if n > l => Standing::Newer,
        (None, Some(_)) => Standing::Newer,
        _ => Standing::Behind,
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
