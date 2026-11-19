//! A read-only DHT transaction's get ([`DhtBackend::get_newest`]), as the
//! pinned veilid-core runs one (`~/veilid-wt-058/veilid-core/src/storage_manager`):
//!
//! - the begin needs the record open and the node online
//!   (`transaction.rs:183-186,206-208`), and every node the fanout reaches
//!   accepts it, since we send the descriptor (`transaction_begin.rs:290-301`);
//!   it aims for `set_value_count` plus [`TRIM_PERCENT`] accepters
//!   (`:398-402`) among the closest nodes, and fails `TryAgain` below
//!   `set_value_count` (`veilid_api/api.rs:700-706`);
//! - each accepter answers its seqs, and the begin keeps the newest
//!   (`transaction_begin.rs:338-347`);
//! - the get asks only the accepters whose begin seq is the newest, and
//!   needs `get_value_count` of them or every one when fewer
//!   (`outbound_transaction_manager/mod.rs:1947-1966`); a seq at the begin
//!   with no value back is `TryAgain` (`transaction.rs:1562-1572`);
//! - it consumes the subkey's watch report mark like any read
//!   (`transaction.rs:1537-1539`) and stores nothing locally
//!   (`:1541-1582`); the rollback changes nothing on the replicas.
//!
//! [`DhtBackend::get_newest`]: rekindle_records::port::DhtBackend::get_newest

use rekindle_records::port::{DhtCallError, RecordValue};

use super::super::{reachable, replicas, stores};
use super::{try_again, SimDht};
use crate::latency;

/// Veilid's `CONSENSUS_TRIM_PERCENT` (`storage_manager/constants.rs:35`).
const TRIM_PERCENT: usize = 50;

impl SimDht {
    /// Begin, get and roll back one subkey of `key`.
    pub(super) async fn transaction_get(
        &self,
        key: &str,
        subkey: u32,
    ) -> Result<Option<RecordValue>, DhtCallError> {
        self.opened(key)?;
        let begin_cap = replicas::SET_COUNT + (replicas::SET_COUNT * TRIM_PERCENT).div_ceil(100);
        let _begin = self.network(latency::GET, key, &[]).await?;
        let accepters: Vec<usize> = {
            let inner = self.net.inner.lock();
            inner.records.get(key).map_or_else(Vec::new, |r| {
                let mut reach = reachable(&r.replicas, &inner.faults.partitions, self.node);
                reach.truncate(begin_cap);
                reach
            })
        };
        if accepters.len() < replicas::SET_COUNT {
            return Err(try_again());
        }
        let _get = self
            .network(self.read_span(key, Some(&[subkey])), key, &[subkey])
            .await?;
        let mut inner = self.net.inner.lock();
        stores::touch(&mut inner, key, &accepters);
        super::super::outbound::read_back(&mut inner, self.node, key, subkey);
        let Some(record) = inner.records.get(key) else {
            return Ok(None);
        };
        let Some(newest) = replicas::newest_seq(&record.replicas, &accepters, subkey as usize)
        else {
            return Ok(None);
        };
        let holders: Vec<usize> = (accepters.iter().copied())
            .filter(|r| {
                (record.replicas.get(*r))
                    .and_then(|x| x.values.get(subkey as usize).cloned().flatten())
                    .is_some_and(|v| v.seq == newest)
            })
            .collect();
        replicas::settle(
            &record.schema,
            &record.replicas,
            &holders,
            subkey as usize,
            None,
        )
        .map(|found| Some(found.value))
        .ok_or_else(try_again)
    }
}
