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
//! - it never delays or refuses a writer: an inbound begin holds the
//!   replica's record lock only while it snapshots
//!   (`record_store/mod.rs:654-693`), as a set holds it only while it stores
//!   (`:437-460`), and the end, not the set, checks the snapshot
//!   (`inbound_transactions_interface.rs:319-330`); what it consumes is one
//!   of the record's [`PUBLIC_TRANSACTION_LIMIT`] anonymous slots on each
//!   accepter until the rollback, a member writer's own transactions being
//!   counted apart (`inbound_transactions_interface.rs:100-133`); a full
//!   replica answers `TransactionUnavailable` and the begin's fanout goes
//!   on to the next node (`transaction_begin.rs:277`).
//!
//! [`DhtBackend::get_newest`]: rekindle_records::port::DhtBackend::get_newest

use rekindle_records::port::{DhtCallError, RecordValue};

use super::super::{reachable, replicas, stores};
use super::{try_again, SimDht};
use crate::latency;

/// Veilid's `CONSENSUS_TRIM_PERCENT` (`storage_manager/constants.rs:35`).
const TRIM_PERCENT: usize = 50;
/// Anonymous transactions a replica holds open per record
/// (`public_transaction_limit`, `veilid_config.rs:1021`).
pub(crate) const PUBLIC_TRANSACTION_LIMIT: u32 = 8;

/// The anonymous slots one transaction holds, released at its rollback or
/// when it fails (stores, not placement indices, which a re-placement moves).
struct OpenRead<'a> {
    dht: &'a SimDht,
    key: &'a str,
    stores: Vec<usize>,
}

impl Drop for OpenRead<'_> {
    fn drop(&mut self) {
        let mut inner = self.dht.net.inner.lock();
        if let Some(record) = inner.records.get_mut(self.key) {
            for r in record
                .replicas
                .iter_mut()
                .filter(|r| self.stores.contains(&r.store))
            {
                r.open_reads = r.open_reads.saturating_sub(1);
            }
        }
    }
}

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
            let mut guard = self.net.inner.lock();
            let inner = &mut *guard;
            let faults = &inner.faults;
            inner.records.get_mut(key).map_or_else(Vec::new, |r| {
                let mut reach = reachable(&r.replicas, faults, self.node);
                reach.retain(|i| r.replicas[*i].open_reads < PUBLIC_TRANSACTION_LIMIT);
                reach.truncate(begin_cap);
                if reach.len() >= replicas::SET_COUNT {
                    for i in &reach {
                        r.replicas[*i].open_reads += 1;
                    }
                }
                reach
            })
        };
        if accepters.len() < replicas::SET_COUNT {
            return Err(try_again());
        }
        let _open = OpenRead {
            dht: self,
            key,
            stores: self.stores_of(key, &accepters),
        };
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

    /// The stores beside the placement indices `of` in `key`.
    fn stores_of(&self, key: &str, of: &[usize]) -> Vec<usize> {
        let inner = self.net.inner.lock();
        (inner.records.get(key)).map_or_else(Vec::new, |r| {
            of.iter()
                .filter_map(|i| r.replicas.get(*i).map(|x| x.store))
                .collect()
        })
    }
}
