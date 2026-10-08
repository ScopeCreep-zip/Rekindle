//! Read by index, never by probe (plan C7.12).
//!
//! A reader of a multi-writer record (a channel's member slots, the member
//! registry) does not read every subkey that could hold a value: in
//! veilid-core 0.5.7 a get with no local value goes to the network even
//! without `force_refresh` (`storage_manager/get_value.rs:72-111`), so each
//! never-written subkey costs a full fanout that ends only at Veilid's
//! timeout. Instead, like every comparable system (Matrix's sync token,
//! Session's `last_hash`, SSB's per-feed vector, VeilidChat's spine head and
//! per-author position), it first takes an index and then reads the gap:
//!
//! 1. The caller names the writers: the subkeys its membership state vouches
//!    for, not the whole range. An inspect answer carries every subkey's seq
//!    at once (`operation_inspect_value.rs:4-6`), but a never-written subkey
//!    answers `None`, which never reaches consensus, so a range padded with
//!    empty slots can never finish its fanout early (`inspect_record.rs`,
//!    `check_done`).
//! 2. One `UpdateGet` inspect over those subkeys gives each one's network seq
//!    beside the local seq Veilid keeps on disk: the per-writer high-water
//!    vector, and our cursor.
//! 3. A subkey is read from the network only when the network holds a newer
//!    seq; one whose local copy is current is a local read with no fanout;
//!    one with no value anywhere is not read.

use futures::stream::{FuturesUnordered, StreamExt};
use rekindle_records::lease::{LeaseId, SubkeySet};
use veilid_core::{DHTReportScope, ValueData, ValueSeqNum};

use super::RecordPool;
use crate::ProtocolError;

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
    ) -> Result<Vec<(u32, ValueData)>, ProtocolError> {
        if subkeys.is_empty() {
            return Ok(Vec::new());
        }
        let set: SubkeySet = subkeys.iter().copied().collect();
        let report = self
            .inspect(id, Some(set), DHTReportScope::UpdateGet)
            .await?;
        let seqs: Vec<(u32, Option<u32>, Option<u32>)> = report
            .subkeys()
            .iter()
            .zip(report.local_seqs().iter().zip(report.network_seqs()))
            .map(|(subkey, (local, network))| {
                (
                    subkey,
                    ValueSeqNum::to_option(local),
                    ValueSeqNum::to_option(network),
                )
            })
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
