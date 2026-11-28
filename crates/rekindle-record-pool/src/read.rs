//! Read by index, never by probe (plan C7.12), and a hint by the network
//! (plan VU3).
//!
//! A reader of a multi-writer record (a channel's member slots, the member
//! registry) does not read every subkey that could hold a value: a get with
//! no local value goes to the network even without `force_refresh`
//! (0.5.8 `storage_manager/get_value.rs:161-169`), so each never-written
//! subkey costs a full fanout that ends only when every node is exhausted
//! or Veilid's timeout passes. Instead, like every comparable system (Matrix's sync token,
//! Session's `last_hash`, SSB's per-feed vector, VeilidChat's spine head and
//! per-author position), it first takes an index and then reads the gap:
//!
//! 1. The caller names the writers: the subkeys its membership state vouches
//!    for, not the whole range. An inspect answer carries every subkey's seq
//!    at once (`operation_inspect_value.rs:4-6`). 0.5.8 ends an inspect once
//!    some subkey has consensus, none is part-converged and the last round
//!    added nothing (`inspect_record.rs:97-121`, `inspect_fanout_done`), so
//!    padding no longer holds it to the timeout when one named subkey is
//!    written; but every padded subkey still costs wire on every node, and a
//!    range nobody ever wrote still runs to exhaustion. The index stays.
//! 2. One `UpdateGet` inspect over those subkeys gives each one's network seq
//!    beside the local seq Veilid keeps on disk: the per-writer high-water
//!    vector, and our cursor.
//! 3. A subkey is read from the network only when the network holds a newer
//!    seq; one whose local copy is current is a local read with no fanout;
//!    one with no value anywhere is not read.
//!
//! An inspect no node answered is `TryAgain` in 0.5.8
//! (`inspect_record.rs:347-349`; 0.5.7 returned a report whose network seqs
//! were all empty, which this read took as "nothing newer" and answered
//! from the local copy, stale and silent). The pool retries it within its
//! budget and then reports [`PortError::Unreachable`], so the caller keeps
//! the copy it has, knowing it missed.
//!
//! A value change is the exception ([`RecordPool::read_hinted`]): Veilid
//! suppresses a later range-only report of a subkey it reported until the
//! app reads that subkey from the network (report marks,
//! `outbound_watch_state.rs:251-297`, cleared only by a network get,
//! `get_value.rs:199-200`). A hinted subkey is therefore always read with
//! `force_refresh`, never planned from an inspect: an inspect that reached
//! a replica not yet caught up would turn it into a local read and leave
//! the mark.
//!
//! An inspect's network seq is unauthenticated (threat model D8), so a
//! claim that the forced get does not confirm is remembered and capped
//! (`refetch.rs`): the same or a lower claim is never fetched again, a
//! higher one once per backoff window. A capped subkey is read locally when
//! we hold a copy, and not at all otherwise (a get with no local value
//! would go to the network anyway). [`RecordPool::read_hinted`] is not
//! capped.

use std::collections::HashMap;

use futures::stream::{FuturesUnordered, StreamExt};
use rekindle_records::lease::{LeaseId, SubkeySet};
use rekindle_records::port::{RecordValue, ReportScope};

use super::RecordPool;
use rekindle_records::port::PortError;

/// Concurrent gets one [`RecordPool::read_changed`] runs.
const READ_PARALLELISM: usize = 10;

/// Which subkeys to read, and which of them from the network: for each
/// `(subkey, local seq, network seq)`, `Some(force_refresh)` when a value
/// exists anywhere, `None` when there is nothing to read.
fn plan_reads(seqs: impl IntoIterator<Item = (u32, Option<u32>, Option<u32>)>) -> Vec<(u32, bool)> {
    seqs.into_iter()
        .filter_map(|(subkey, local, network)| {
            if local.is_none() && network.is_none() {
                return None;
            }
            Some((subkey, network > local))
        })
        .collect()
}

impl RecordPool {
    /// The values of `subkeys` on the leased record, reading only what an
    /// inspect says exists and fetching from the network only what changed.
    /// A subkey whose read fails is left out, as one with no value is.
    ///
    /// # Errors
    /// The inspect failed (the lease is not held, or the node is offline):
    /// the caller keeps the copy it already has.
    pub async fn read_changed(
        &self,
        id: LeaseId,
        subkeys: &[u32],
    ) -> Result<Vec<(u32, RecordValue)>, PortError> {
        if subkeys.is_empty() {
            return Ok(Vec::new());
        }
        let set: SubkeySet = subkeys.iter().copied().collect();
        let report = self.inspect(id, Some(set), ReportScope::UpdateGet).await?;
        let seqs: Vec<(u32, Option<u32>, Option<u32>)> = report
            .subkeys
            .iter()
            .zip(report.local_seqs.iter().zip(&report.network_seqs))
            .map(|(subkey, (local, network))| (*subkey, *local, *network))
            .collect();
        tracing::debug!(
            key = self.key_of(id).as_deref().unwrap_or("?"),
            asked = subkeys.len(),
            present = ?seqs
                .iter()
                .filter(|(_, local, network)| local.is_some() || network.is_some())
                .collect::<Vec<_>>(),
            "read_changed: inspected (subkey, local seq, network seq)",
        );
        let plan = self.capped(id, &seqs, plan_reads(seqs.iter().copied()));

        let sem = tokio::sync::Semaphore::new(READ_PARALLELISM);
        let mut reads = FuturesUnordered::new();
        for (subkey, claimed) in plan {
            let sem = &sem;
            reads.push(async move {
                let _permit = sem.acquire().await;
                (
                    subkey,
                    claimed,
                    self.get(id, subkey, claimed.is_some()).await,
                )
            });
        }
        let mut values = Vec::new();
        while let Some((subkey, claimed, result)) = reads.next().await {
            self.settle_claim(id, subkey, claimed, &result);
            match result {
                Ok(Some(value)) => values.push((subkey, value)),
                Ok(None) => {}
                Err(error) => {
                    tracing::debug!(subkey, %error, "read_changed: subkey read failed");
                }
            }
        }
        values.sort_by_key(|(subkey, _)| *subkey);
        Ok(values)
    }

    /// `plan` with D8's cap applied (`refetch.rs`): each read with the
    /// network seq it fetches for (`Some` for a network read). A network
    /// read of a capped claim becomes a local read, or no read when there
    /// is no local copy.
    fn capped(
        &self,
        id: LeaseId,
        seqs: &[(u32, Option<u32>, Option<u32>)],
        plan: Vec<(u32, bool)>,
    ) -> Vec<(u32, Option<u32>)> {
        let Some(key) = self.key_of(id) else {
            return plan.into_iter().map(|(s, _)| (s, None)).collect();
        };
        let known: HashMap<u32, (Option<u32>, Option<u32>)> =
            seqs.iter().map(|(s, l, n)| (*s, (*l, *n))).collect();
        let now = tokio::time::Instant::now();
        let unconfirmed = self.unconfirmed.lock();
        plan.into_iter()
            .filter_map(|(subkey, force_refresh)| {
                let (local, network) = known.get(&subkey).copied().unwrap_or_default();
                let claimed = network.filter(|_| force_refresh);
                match claimed {
                    Some(seq) if !unconfirmed.may_fetch(&key, subkey, seq, now) => {
                        tracing::debug!(
                            key,
                            subkey,
                            claimed = seq,
                            ?local,
                            "read_changed: an unconfirmed inspect seq is capped (D8)",
                        );
                        local.map(|_| (subkey, None))
                    }
                    _ => Some((subkey, claimed)),
                }
            })
            .collect()
    }

    /// Record what a get of `subkey` said about a claim: `claimed` is the
    /// network seq a forced read fetched for, `None` for any other get
    /// (which still clears a claim its value reaches).
    fn settle_claim(
        &self,
        id: LeaseId,
        subkey: u32,
        claimed: Option<u32>,
        result: &Result<Option<RecordValue>, PortError>,
    ) {
        let Some(key) = self.key_of(id) else {
            return;
        };
        let got = result.as_ref().ok().and_then(|v| v.as_ref().map(|v| v.seq));
        let mut unconfirmed = self.unconfirmed.lock();
        match (claimed, got) {
            (Some(claimed), got) => {
                unconfirmed.fetched(&key, subkey, claimed, got, tokio::time::Instant::now());
            }
            (None, Some(seq)) => unconfirmed.confirmed(&key, subkey, seq),
            (None, None) => {}
        }
    }

    /// Retry a hinted read using D8's existing unconfirmed-claim cap.
    pub(crate) async fn refetch_hinted(
        &self,
        id: LeaseId,
        subkey: u32,
        claimed: u64,
    ) -> Result<Option<RecordValue>, PortError> {
        let Ok(claimed) = u32::try_from(claimed) else {
            return Ok(None);
        };
        let key = self.key_of(id).ok_or(PortError::LeaseNotHeld(id.0))?;
        if !self
            .unconfirmed
            .lock()
            .may_fetch(&key, subkey, claimed, tokio::time::Instant::now())
        {
            return Ok(None);
        }
        let result = self.get_publishing(id, subkey, true, false).await;
        self.settle_claim(id, subkey, Some(claimed), &result);
        result.map(|value| value.filter(|value| value.seq >= claimed))
    }

    /// [`read_changed`](Self::read_changed)'s hinted mode: every subkey a
    /// value change named, read from the network so its report mark clears
    /// (see the module notes). A subkey whose read fails or finds no value
    /// is left out.
    ///
    /// # Errors
    /// Every read failed (the lease is not held, or the node is offline):
    /// the caller keeps the copy it already has.
    pub async fn read_hinted(
        &self,
        id: LeaseId,
        subkeys: &[u32],
    ) -> Result<Vec<(u32, RecordValue)>, PortError> {
        let set: SubkeySet = subkeys.iter().copied().collect();
        let sem = tokio::sync::Semaphore::new(READ_PARALLELISM);
        let mut reads = FuturesUnordered::new();
        for subkey in set {
            let sem = &sem;
            reads.push(async move {
                let _permit = sem.acquire().await;
                (subkey, self.get_publishing(id, subkey, true, false).await)
            });
        }
        let mut values = Vec::new();
        let mut last_error = None;
        let mut answered = false;
        while let Some((subkey, result)) = reads.next().await {
            self.settle_claim(id, subkey, None, &result);
            match result {
                Ok(value) => {
                    answered = true;
                    values.extend(value.map(|v| (subkey, v)));
                }
                Err(error) => {
                    tracing::debug!(subkey, %error, "read_hinted: subkey read failed");
                    last_error = Some(error);
                }
            }
        }
        match last_error {
            Some(error) if !answered => Err(error),
            _ => {
                values.sort_by_key(|(subkey, _)| *subkey);
                Ok(values)
            }
        }
    }
}

#[cfg(test)]
#[path = "read_tests.rs"]
mod tests;
