//! Change inspection: the watcher's catch-up (plan E2.B slice S6: M2),
//! after the pinned veilid-core (`~/veilid/veilid-core/src/storage_manager`):
//!
//! - **when:** a node's watch is queued for inspection when its DHT comes
//!   back online (`routing_domain_ready_event_handler`, `mod.rs:887-898` →
//!   `change_inspect_all_watches`, `watch_value.rs:2019-2041`), when the
//!   storage manager's tick ran more than [`CHANGE_INSPECT_LAG`] late, at
//!   most once per [`CHANGE_INSPECT_LAG_MIN_INTERVAL`] (`tasks/mod.rs:132-145`,
//!   `constants.rs:45-47`), and when the watch's parameters change or a
//!   replica newly holds it (`watch_value.rs:1166-1171`). A queued
//!   inspection runs at the next watch tick (`check_outbound_watches`);
//!   one that fails (offline) is queued again after
//!   [`CHANGE_INSPECT_RETRY_INTERVAL`] while the watch exists
//!   (`watch_value.rs:1324-1340`, `constants.rs:49`);
//! - **what:** an inspect of the watched subkeys over the replicas the node
//!   reaches gives the subkeys where the network is ahead of the local copy
//!   (`newer_online_subkeys`); from the first of them, a forced get stores
//!   the network's value locally, and the first whose value is newer than
//!   the local seq is reported, with the subkeys still in the set (that one
//!   first), the watch's count less one, and that subkey's value
//!   (`watch_value.rs:1286-1318`, `report_change_inspect_value` `:1349-1403`).
//!   Nothing newer reports nothing.
//!
//! The local copy moves when the tick runs; the change arrives after the
//! inspect's and the get's latency ([`crate::latency::GET`] each).

use std::collections::BTreeSet;
use std::time::Duration;

use rekindle_records::port::{PortEvent, ValueChange};

use super::replicas::{self, newest_seq};
use super::{reachable, watches};
use crate::latency::{self, Latency};
use crate::network::{Inner, Network};

/// `CHANGE_INSPECT_LAG` (`constants.rs:45`).
pub(crate) const CHANGE_INSPECT_LAG: Duration = Duration::from_secs(2);
/// `CHANGE_INSPECT_LAG_MIN_INTERVAL` (`constants.rs:47`).
pub(crate) const CHANGE_INSPECT_LAG_MIN_INTERVAL: Duration = Duration::from_secs(30);
/// `CHANGE_INSPECT_RETRY_INTERVAL` (`constants.rs:49`).
pub(crate) const CHANGE_INSPECT_RETRY_INTERVAL: Duration = Duration::from_secs(5);

fn micros(d: Duration) -> u64 {
    u64::try_from(d.as_micros()).unwrap_or(u64::MAX)
}

/// Queue an inspection of `node`'s watch on `key`, not before `not_before`
/// (`enqueue_change_inspect`; one queued per record, the soonest kept).
pub(crate) fn enqueue(inner: &mut Inner, node: usize, key: &str, not_before: u64) {
    let at = inner.nodes[node]
        .inspects
        .entry(key.to_string())
        .or_insert(not_before);
    *at = (*at).min(not_before);
}

/// `change_inspect_all_watches`: queue every live watch of `node`.
fn inspect_all(inner: &mut Inner, node: usize, now: u64) {
    let mut keys: Vec<String> = inner.nodes[node]
        .watches
        .iter()
        .filter(|(_, d)| d.is_some())
        .map(|(k, _)| k.clone())
        .collect();
    keys.sort();
    for key in keys {
        enqueue(inner, node, &key, now);
    }
}

/// Whether `node` came back online since the last look (Veilid's
/// `is_online` swap), queueing every watch when it did.
pub(super) fn readiness(inner: &mut Inner, node: usize, now: u64) {
    let on = watches::online(inner, node);
    let was = std::mem::replace(&mut inner.nodes[node].was_online, on);
    if on && !was {
        inspect_all(inner, node, now);
    }
}

/// Whether `node` has catch-up work a tick can make progress on: a
/// readiness edge to take, or an inspection queued while it is online (an
/// offline node's retries change nothing until it is back, so they do not
/// keep the network busy).
pub(super) fn pending(inner: &Inner, node: usize) -> bool {
    let slot = &inner.nodes[node];
    let on = watches::online(inner, node);
    (on && !slot.inspects.is_empty()) || (slot.was_online != on && !slot.watches.is_empty())
}

/// Run `node`'s due inspections; each change found with its delay.
pub(super) fn run(
    inner: &mut Inner,
    latency: &Latency,
    node: usize,
    now: u64,
) -> Vec<(ValueChange, Duration)> {
    let mut due: Vec<String> = inner.nodes[node]
        .inspects
        .iter()
        .filter(|(_, at)| **at <= now)
        .map(|(k, _)| k.clone())
        .collect();
    due.sort();
    let mut found = Vec::new();
    for key in due {
        inner.nodes[node].inspects.remove(&key);
        if !matches!(inner.nodes[node].watches.get(&key), Some(Some(_))) {
            continue;
        }
        if !watches::online(inner, node) {
            enqueue(
                inner,
                node,
                &key,
                now.saturating_add(micros(CHANGE_INSPECT_RETRY_INTERVAL)),
            );
            continue;
        }
        if let Some(change) = inspect(inner, node, &key) {
            let delay = latency.draw(node, latency::GET) + latency.draw(node, latency::GET);
            found.push((change, delay));
        }
    }
    found
}

/// One inspection of `node`'s watch on `key` (`watch_value.rs:1286-1318`).
fn inspect(inner: &mut Inner, node: usize, key: &str) -> Option<ValueChange> {
    let subkeys = inner.nodes[node]
        .watches
        .get(key)?
        .as_ref()?
        .subkeys
        .clone();
    let reach = reachable(
        &inner.records.get(key)?.replicas,
        &inner.faults.partitions,
        node,
    );
    super::stores::touch(inner, key, &reach);
    let record = inner.records.get(key)?;
    let mut newer: BTreeSet<u32> = {
        let local = &inner.nodes[node].local;
        subkeys
            .into_iter()
            .filter(|s| {
                let held = local.get(&(key.to_string(), *s)).map(|v| v.seq);
                newest_seq(&record.replicas, &reach, *s as usize) > held
            })
            .collect()
    };
    while let Some(first) = newer.first().copied() {
        let held = inner.nodes[node]
            .local
            .get(&(key.to_string(), first))
            .cloned();
        let got = replicas::get(
            &record.schema,
            &record.replicas,
            &reach,
            first as usize,
            held.as_ref(),
        );
        if let Some(value) = got {
            let local = &mut inner.nodes[node].local;
            if held.as_ref().is_none_or(|l| l.seq != value.seq) {
                local.insert((key.to_string(), first), value.clone());
            }
            if held.is_none_or(|l| value.seq > l.seq) {
                let desired = inner.nodes[node].watches.get_mut(key)?.as_mut()?;
                desired.count = desired.count.saturating_sub(1);
                return Some(ValueChange {
                    key: key.to_string(),
                    subkeys: newer.into_iter().collect(),
                    count: desired.count,
                    value: Some(value),
                });
            }
        }
        newer.pop_first();
    }
    None
}

impl Network {
    /// The node's storage-manager tick ran `lag` late (a suspended or
    /// overloaded process): past [`CHANGE_INSPECT_LAG`], and online, every
    /// watch is inspected, at most once per
    /// [`CHANGE_INSPECT_LAG_MIN_INTERVAL`] (`tasks/mod.rs:132-145`).
    pub(crate) fn tick_lag(&self, node: usize, lag: Duration) {
        let now = self.now_us();
        let mut inner = self.inner.lock();
        if node >= inner.nodes.len() || lag <= CHANGE_INSPECT_LAG || !watches::online(&inner, node)
        {
            return;
        }
        let last = inner.nodes[node].last_lag_inspect;
        if last.is_some_and(|t| now < t.saturating_add(micros(CHANGE_INSPECT_LAG_MIN_INTERVAL))) {
            return;
        }
        inner.nodes[node].last_lag_inspect = Some(now);
        inspect_all(&mut inner, node, now);
    }

    /// Send the changes [`run`] found.
    pub(super) fn send_inspected(&self, found: Vec<(usize, ValueChange, Duration)>) {
        for (node, change, delay) in found {
            self.send_after(
                node,
                node,
                "ChangeInspect",
                PortEvent::ValueChange(change),
                false,
                delay,
            );
        }
    }
}
