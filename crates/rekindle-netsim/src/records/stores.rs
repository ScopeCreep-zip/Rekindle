//! Each DHT store's record bound (plan E2.B slice S7, M5), after the pinned
//! veilid-core (`~/veilid/veilid-core/src/storage_manager`):
//!
//! - a remote record store keeps an LRU of its records
//!   (`record_store/mod.rs:2-5`), bounded by `max_records`
//!   (`record_store/record_store_limits.rs:13`), which a remote store takes
//!   from `remote_max_records` (`mod.rs:384`). The replicas here are the
//!   network's DHT nodes, so the bound is Veilid's default for them,
//!   [`REMOTE_MAX_RECORDS`] (`veilid_config.rs:364`); Rekindle raises it only
//!   on its own app nodes, which do not serve the DHT
//!   (`rekindle-protocol/src/veilid_config.rs`). The storage-space bound
//!   (`record_store_limits.rs:17`, 256 MB) is far above what a scenario
//!   stores and is not modeled;
//! - a store uses a record when a call reads or writes it there
//!   (`record_store_inner/record_index.rs:249,504` take it through the
//!   LRU's `get`); a record new to a full store evicts the least recently
//!   used one (`record_index.rs:175-201`), whose values and watches are gone
//!   from that store: a later get misses it there, and a rehydration push
//!   can store it again ([`super::rehydrate`]).

use std::collections::BTreeSet;

use super::replicas::Replica;
use crate::network::Inner;

/// `remote_max_records`'s default (`veilid_config.rs:364`).
pub(crate) const REMOTE_MAX_RECORDS: usize = 128;

fn holds(replica: &Replica) -> bool {
    replica.values.iter().any(Option::is_some)
}

/// The `reach` replicas of `key` were asked: each that holds the record
/// becomes its store's most recently used.
pub(crate) fn touch(inner: &mut Inner, key: &str, reach: &[usize]) {
    inner.lru += 1;
    let now = inner.lru;
    if let Some(record) = inner.records.get_mut(key) {
        for r in reach {
            if let Some(replica) = record.replicas.get_mut(*r).filter(|x| holds(x)) {
                replica.used = now;
            }
        }
    }
}

/// The `stored` replicas of `key` took a value: they are used now, and each
/// of their stores past [`REMOTE_MAX_RECORDS`] evicts its least recently
/// used records.
pub(crate) fn admit(inner: &mut Inner, key: &str, stored: &[usize]) {
    touch(inner, key, stored);
    let stores: BTreeSet<usize> = inner.records.get(key).map_or_else(BTreeSet::new, |record| {
        stored
            .iter()
            .filter_map(|r| record.replicas.get(*r).map(|x| x.store))
            .collect()
    });
    for store in stores {
        while evict(inner, store) {}
    }
}

/// Evict `store`'s least recently used record when it holds more than its
/// bound; whether it did.
fn evict(inner: &mut Inner, store: usize) -> bool {
    let mut held: Vec<(u64, &String, usize)> = inner
        .records
        .iter()
        .flat_map(|(key, record)| {
            record
                .replicas
                .iter()
                .enumerate()
                .filter(|(_, x)| x.store == store && holds(x))
                .map(move |(i, x)| (x.used, key, i))
        })
        .collect();
    if held.len() <= REMOTE_MAX_RECORDS {
        return false;
    }
    held.sort();
    let (key, r) = (held[0].1.clone(), held[0].2);
    if let Some(replica) = inner
        .records
        .get_mut(&key)
        .and_then(|record| record.replicas.get_mut(r))
    {
        replica.values.iter_mut().for_each(|v| *v = None);
        replica.watches.clear();
    }
    true
}
