//! Rehydration (plan E2.B slice S7, M5), after the pinned veilid-core
//! (`~/veilid/veilid-core/src/storage_manager`):
//!
//! - **when:** an open of a record in the node's local store queues a
//!   rehydration of every subkey at `set_value_count` consensus
//!   (`open_record.rs:31-38`, `add_rehydration_request` `rehydrate.rs:23-48`);
//!   a create or an open from the network queues none. A set refused
//!   offline (`allow_offline` false, as the pool always sends) queues
//!   nothing (`local_record_store_interface.rs:61-62`): the push queue is
//!   fed by rehydration only (`rehydrate.rs:223,264`), so it never doubles
//!   the pool's own durable re-push;
//! - **the tick:** every second (`OFFLINE_SUBKEY_WRITES_INTERVAL_SECS`,
//!   `REHYDRATE_RECORDS_INTERVAL_SECS`, `constants.rs:8,37`) the node first
//!   pushes its queued subkeys, then takes up to [`REHYDRATE_PER_TICK`]
//!   requests (`constants.rs:41`, `tasks/mod.rs:119-125`,
//!   `tasks/rehydrate_records.rs:17-28`), in key order for the seed. An
//!   offline node pushes nothing (`tasks/offline_subkey_writes.rs:43-46`)
//!   and keeps its requests (`TryAgain` re-queued, `rehydrate.rs:136-138`,
//!   `rehydrate_records.rs:45-55`);
//! - **which subkeys:** an inspect in the set scope over the replicas the
//!   node reaches (`inspect_record.rs:292-301`): with no reached replica
//!   holding the record, every local subkey; else each local subkey newer
//!   than the network's newest seq, or at it or older with fewer than
//!   [`SET_COUNT`] replicas at that seq (`rehydrate.rs:180-194,306-341`,
//!   `inspect_record.rs:486-522`);
//! - **the push:** a queued subkey is sent as a set of its local copy
//!   (`offline_subkey_writes.rs:47-74`); a replica holding a newer value
//!   restarts the fanout on it ([`super::replicas::set`]), and a value the
//!   fanout ended on that is not ours becomes the local copy (`:84-93`).
//!   Fewer than [`GET_COUNT`] accepts keeps the subkey queued
//!   (`check_fanout_finished_without_consensus`, `mod.rs:781-820`), else it
//!   is done.
//!
//! The push has no answer anything waits on, so its effect lands at the
//! tick. A push left queued whose node reaches the same replicas as at its
//! last attempt is not pending work (the retry is the same fanout), so a cut
//! that keeps it short does not keep the network busy; it still retries at
//! every tick.

use super::replicas::{self, newest_seq, GET_COUNT, SET_COUNT};
use super::{reachable, stores, subkey_count, watches};
use crate::network::{Inner, Network};

/// `REHYDRATE_REQUESTS_PER_TICK` (`constants.rs:41`).
pub(crate) const REHYDRATE_PER_TICK: usize = 4;

/// `add_rehydration_request` for every subkey of `key` on `node`.
pub(super) fn request(inner: &mut Inner, node: usize, key: &str) {
    inner.nodes[node].rehydrations.insert(key.to_string());
}

fn reach_of(inner: &Inner, node: usize, key: &str) -> Vec<usize> {
    inner.records.get(key).map_or_else(Vec::new, |r| {
        reachable(&r.replicas, &inner.faults.partitions, node)
    })
}

/// Whether `node` has rehydration work a tick can make progress on.
pub(super) fn pending(inner: &Inner, node: usize) -> bool {
    let slot = &inner.nodes[node];
    watches::online(inner, node)
        && (!slot.rehydrations.is_empty()
            || slot.pushes.iter().any(|((key, _), last)| {
                last.as_ref()
                    .is_none_or(|l| *l != reach_of(inner, node, key))
            }))
}

/// One rehydration of `key` on `node`: queue the subkeys to push.
fn rehydrate(inner: &mut Inner, node: usize, key: &str) {
    let reach = reach_of(inner, node, key);
    let Some(record) = inner.records.get(key) else {
        return;
    };
    let online = replicas::holds(&record.replicas, &reach, None);
    let local = &inner.nodes[node].local;
    let needed: Vec<u32> = (0..subkey_count(&record.schema))
        .filter(|s| {
            let Some(held) = local.get(&(key.to_string(), *s)) else {
                return false;
            };
            let net = newest_seq(&record.replicas, &reach, *s as usize);
            let at_newest = reach
                .iter()
                .filter_map(|r| record.replicas.get(*r)?.values.get(*s as usize)?.as_ref())
                .filter(|v| Some(v.seq) == net)
                .count();
            !online || Some(held.seq) > net || at_newest < SET_COUNT
        })
        .collect();
    stores::touch(inner, key, &reach);
    for subkey in needed {
        inner.nodes[node]
            .pushes
            .insert((key.to_string(), subkey), None);
    }
}

/// One push of `node`'s local copy of `key`'s `subkey`.
fn push(inner: &mut Inner, node: usize, key: &str, subkey: u32) {
    let local = (key.to_string(), subkey);
    let Some(ours) = inner.nodes[node].local.get(&local).cloned() else {
        return;
    };
    let reach = reach_of(inner, node, key);
    let Some(record) = inner.records.get_mut(key) else {
        return;
    };
    let fanout = replicas::set(&mut record.replicas, &reach, subkey as usize, &ours);
    watches::mark(record, &fanout.stored, subkey, node);
    stores::touch(inner, key, &reach);
    stores::admit(inner, key, &fanout.stored);
    if fanout.value != ours {
        inner.nodes[node]
            .local
            .insert(local.clone(), fanout.value.clone());
    }
    if fanout.accepted < GET_COUNT {
        inner.nodes[node].pushes.insert(local, Some(reach));
    }
}

impl Network {
    /// The storage manager's offline-write and rehydration tasks, for every
    /// node (see the module notes).
    pub(crate) fn tick_rehydration(&self) {
        let mut guard = self.inner.lock();
        let inner = &mut *guard;
        for node in 0..inner.nodes.len() {
            if !watches::online(inner, node) {
                continue;
            }
            let queued = std::mem::take(&mut inner.nodes[node].pushes);
            for (key, subkey) in queued.into_keys() {
                push(inner, node, &key, subkey);
            }
            let due: Vec<String> = inner.nodes[node]
                .rehydrations
                .iter()
                .take(REHYDRATE_PER_TICK)
                .cloned()
                .collect();
            for key in due {
                inner.nodes[node].rehydrations.remove(&key);
                rehydrate(inner, node, &key);
            }
        }
    }
}
