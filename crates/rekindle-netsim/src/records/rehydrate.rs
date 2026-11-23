//! Rehydration (plan E2.B slice S7, M5), after the pinned veilid-core
//! (`~/veilid-wt-058/veilid-core/src/storage_manager`):
//!
//! - **when:** an open of a record in the node's local store queues a
//!   rehydration of every subkey at `set_value_count` consensus, when the
//!   open asks for it, the node did not create the record this session and
//!   its last check is past [`REHYDRATE_RECHECK`] ([`on_open`]); a create or
//!   an open from the network queues none. A set refused
//!   offline (`allow_offline` false, as the pool always sends) queues
//!   nothing (0.5.8 `local_record_store_interface.rs:61-62`): the push queue is
//!   fed by rehydration only (`rehydrate.rs:284,325`), so it never doubles
//!   the pool's own durable re-push;
//! - **the tick:** every second (`OFFLINE_SUBKEY_WRITES_INTERVAL_SECS`,
//!   `REHYDRATE_RECORDS_INTERVAL_SECS`, `constants.rs:8,37`) the node first
//!   pushes its queued subkeys, then takes up to [`REHYDRATE_PER_TICK`]
//!   requests (`constants.rs:41`, `tasks/mod.rs:146-152`,
//!   `tasks/rehydrate_records.rs:17-29`), in key order for the seed. An
//!   offline node pushes nothing (`tasks/offline_subkey_writes.rs:43-46`)
//!   and keeps its requests (`TryAgain` re-queued, `rehydrate.rs:176-177`,
//!   `rehydrate_records.rs:56-64`);
//! - **which subkeys:** an inspect in the set scope over the replicas the
//!   node reaches (`inspect_record.rs:425-434`): with no reached replica
//!   holding the record, every local subkey; else each local subkey newer
//!   than the network's seq, or at it or older with fewer than
//!   [`SET_COUNT`] consensus nodes at that seq (`rehydrate.rs:241-255,371-405`).
//!   The inspect starts from nothing and stops at [`SET_COUNT`] agreeing
//!   replicas (`rehydrate.rs:197-206`, [`replicas::consensus`]), so a newer
//!   seq answered after it is not seen. A replica with no value at a subkey
//!   is no evidence for it: it counts toward no seq
//!   (`inspect_record.rs:638-652,695-705`);
//! - **the push:** a queued subkey is sent as a set of its local copy
//!   (`tasks/offline_subkey_writes.rs:47-99`); a replica holding a newer value
//!   restarts the fanout on it ([`super::fanout`]), and a value the
//!   fanout ended on that is not ours becomes the local copy (`:111-119`).
//!   A push that did not land (fewer than
//!   [`super::replicas::GET_COUNT`] accepts, [`super::fanout`]) keeps the subkey queued
//!   (`check_fanout_finished_without_consensus`, `mod.rs:990-1029`), else it
//!   is done.
//!
//! The push has no answer anything waits on, so its effect lands at the
//! tick. A push left queued whose node reaches the same replicas as at its
//! last attempt is not pending work (the retry is the same fanout), so a cut
//! that keeps it short does not keep the network busy; it still retries at
//! every tick.

use super::replicas::{self, SET_COUNT};
use super::{candidates, reachable, stores, subkey_count, watches};
use crate::network::{Inner, Network};

/// `REHYDRATE_REQUESTS_PER_TICK` (`constants.rs:41`).
pub(crate) const REHYDRATE_PER_TICK: usize = 4;

/// How long a completed rehydration check answers for a record: 60 RPC
/// timeouts (`storage_manager/mod.rs:198`, `rpc.timeout_ms` 5 000).
pub(crate) const REHYDRATE_RECHECK: std::time::Duration = std::time::Duration::from_secs(300);

/// An open of `key`, held in `node`'s local store, with the `rehydrate`
/// option: Veilid queues a rehydration of every subkey only when asked, for
/// a record this node did not create this session, whose last check is
/// older than [`REHYDRATE_RECHECK`] (0.5.8 `open_record.rs:62-81`,
/// `rehydrate.rs:26-37`).
pub(super) fn on_open(inner: &mut Inner, node: usize, key: &str, rehydrate: bool) {
    let slot = &inner.nodes[node];
    let due = slot
        .rehydration_checked
        .get(key)
        .is_none_or(|at| tokio::time::Instant::now() >= *at + REHYDRATE_RECHECK);
    if rehydrate && !slot.created.contains(key) && due {
        inner.nodes[node].rehydrations.insert(key.to_string());
    }
}

fn reach_of(inner: &Inner, node: usize, key: &str) -> Vec<usize> {
    inner
        .records
        .get(key)
        .map_or_else(Vec::new, |r| reachable(&r.replicas, &inner.faults, node))
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

/// One rehydration of `key` on `node`: queue the subkeys to push, and mark
/// the record checked. With no replica reached its inspect answers
/// `TryAgain` (0.5.8 `inspect_record.rs:346-349`) and the request stays
/// queued (`tasks/rehydrate_records.rs:56-64`). A record the node's local
/// store does not hold answers `KeyNotFound` (0.5.8 `rehydrate.rs:155-165`)
/// and its request is dropped, unchecked (`rehydrate_records.rs:65-66`).
/// The `TryAgain` of an inspect pre-empted by foreground work
/// (`rehydrate.rs:197-211`) cannot happen here: the inspect takes no
/// simulated time, so no foreground call overlaps it.
fn rehydrate(inner: &mut Inner, node: usize, key: &str) {
    if !inner.nodes[node].known.contains(key) {
        inner.nodes[node].rehydrations.remove(key);
        return;
    }
    let reach = reach_of(inner, node, key);
    if reach.is_empty() {
        return;
    }
    inner.nodes[node].rehydrations.remove(key);
    inner.nodes[node]
        .rehydration_checked
        .insert(key.to_string(), tokio::time::Instant::now());
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
            let (net, agreeing) =
                replicas::consensus(&record.replicas, &reach, *s as usize, None, SET_COUNT);
            !online || Some(held.seq) > net || agreeing < SET_COUNT
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
    let Some(ours) = super::replicas::local(inner, node, key, subkey) else {
        return;
    };
    let reach = reach_of(inner, node, key);
    let Some(record) = inner.records.get_mut(key) else {
        return;
    };
    let candidates = candidates(&record.replicas, &inner.faults, node);
    let fanout = super::fanout::set_now(
        &record.schema,
        &mut record.replicas,
        &candidates,
        subkey as usize,
        &ours,
    );
    watches::mark(record, &fanout.stored, subkey);
    stores::touch(inner, key, &reach);
    stores::admit(inner, key, &fanout.stored);
    if fanout.value != ours {
        super::replicas::keep(inner, node, key, subkey, &fanout.value);
    }
    if !fanout.landed() {
        inner.nodes[node]
            .pushes
            .insert((key.to_string(), subkey), Some(reach));
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
                rehydrate(inner, node, &key);
            }
        }
    }
}
