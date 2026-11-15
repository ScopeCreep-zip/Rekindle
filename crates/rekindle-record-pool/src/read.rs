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
        let plan = plan_reads(seqs);

        let sem = tokio::sync::Semaphore::new(READ_PARALLELISM);
        let mut reads = FuturesUnordered::new();
        for (subkey, force_refresh) in plan {
            let sem = &sem;
            reads.push(async move {
                let _permit = sem.acquire().await;
                (subkey, self.get(id, subkey, force_refresh).await)
            });
        }
        let mut values = Vec::new();
        while let Some((subkey, result)) = reads.next().await {
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
                (subkey, self.get(id, subkey, true).await)
            });
        }
        let mut values = Vec::new();
        let mut last_error = None;
        let mut answered = false;
        while let Some((subkey, result)) = reads.next().await {
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
mod tests {
    use super::plan_reads;

    #[test]
    fn a_subkey_with_no_value_anywhere_is_not_read() {
        assert!(plan_reads([(4, None, None)]).is_empty());
    }

    #[test]
    fn a_current_local_copy_is_read_locally() {
        assert_eq!(plan_reads([(4, Some(3), Some(3))]), vec![(4, false)]);
        // The network answered nothing newer (or nothing): ours stands.
        assert_eq!(plan_reads([(5, Some(3), None)]), vec![(5, false)]);
    }

    #[test]
    fn only_a_newer_network_value_is_fetched_from_the_network() {
        assert_eq!(
            plan_reads([
                (1, Some(2), Some(7)),
                (2, None, Some(0)),
                (3, Some(9), Some(4))
            ]),
            vec![(1, true), (2, true), (3, false)]
        );
    }
}
