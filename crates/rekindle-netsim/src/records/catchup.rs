//! Change inspection: the watcher's catch-up (plan E2.B slice S6: M2; plan
//! VU2 item 4), after Veilid 0.5.8 (`~/veilid-wt-058/veilid-core/src/
//! storage_manager`):
//!
//! - **when:** a node's watch is queued for inspection when its DHT comes
//!   back online (`mod.rs:1096-1112` → `change_inspect_all_watches`,
//!   `watch_value.rs:2444-2468`), when its fallback silence timer is due
//!   ([`super::outbound`], `tasks/check_outbound_watches.rs:8-68`), and
//!   when the watch's parameters change or a replica newly holds it
//!   (`watch_value.rs:1317-1325`). 0.5.7's tick-lag sweep is gone (0.5.8
//!   `constants.rs` has no `CHANGE_INSPECT_LAG`). Queued inspections run at
//!   the next watch tick, the oldest first, at most [`MAX_IN_FLIGHT`] at
//!   once (`outbound_watch_manager/mod.rs:371-395`), and none while the
//!   node is offline: the queue holds (`mod.rs:1129-1131`,
//!   `check_outbound_watches.rs:105-173`). One that no replica answers is
//!   queued again after [`CHANGE_INSPECT_RETRY_INTERVAL`], doubling per
//!   failure up to [`MAX_INSPECT_INTERVAL`] (`requeue_failed_change_inspect`,
//!   `watch_value.rs:2403-2442`; `failure_backoff`,
//!   `network_manager/online_detector.rs:117-131`, survival 1);
//! - **what:** an inspect of the watched subkeys over the replicas the node
//!   reaches gives the subkeys where the network is ahead of the local copy
//!   (`newer_online_subkeys`), which backs the silence timer off or resets
//!   it. One newer subkey is read (a network get, stored locally when its
//!   seq moved) and reported with its value when newer than the local seq;
//!   several are reported as the range with no value and no read
//!   (`watch_value.rs:1433-1520`). Each report counts the watch down.
//!   Nothing newer reports nothing.
//!
//! An inspection's answer arrives after an inspect's latency, plus a get's
//! for the one-subkey read ([`crate::latency::GET`] each).

use std::collections::BTreeSet;
use std::time::Duration;

use rekindle_records::port::{PortEvent, ValueChange};

use super::outbound::{self, micros, MAX_INSPECT_INTERVAL};
use super::replicas::{self, newest_seq};
use super::{reachable, watches};
use crate::latency::{self, Latency};
use crate::network::{Inner, Network};

/// `CHANGE_INSPECT_RETRY_INTERVAL` (`constants.rs:45`).
pub(crate) const CHANGE_INSPECT_RETRY_INTERVAL: Duration = Duration::from_secs(5);
/// `MAX_CHANGE_INSPECTIONS_IN_FLIGHT` = `RELAY_WORKERS_PER_CORE`
/// (`constants.rs:47-48`, `network_manager/mod.rs:86`).
pub(crate) const MAX_IN_FLIGHT: usize = 16;

/// Queue an inspection of `node`'s watch on `key`, not before `at` (node
/// clock µs; one queued per record, the soonest kept).
fn enqueue_at(inner: &mut Inner, node: usize, key: &str, at: u64) {
    let queued = inner.nodes[node]
        .outbound
        .queue
        .entry(key.to_string())
        .or_insert(at);
    *queued = (*queued).min(at);
}

/// Queue an inspection of `node`'s watch on `key` now (`now` is true
/// time, µs).
pub(crate) fn enqueue(inner: &mut Inner, node: usize, key: &str, now: u64) {
    let at = outbound::clock(inner, node, now);
    enqueue_at(inner, node, key, at);
}

/// `change_inspect_all_watches`: queue every live watch of `node`.
fn inspect_all(inner: &mut Inner, node: usize, at: u64) {
    let mut keys: Vec<String> = inner.nodes[node]
        .watches
        .iter()
        .filter(|(_, d)| d.is_some())
        .map(|(k, _)| k.clone())
        .collect();
    keys.sort();
    for key in keys {
        enqueue_at(inner, node, &key, at);
    }
}

/// Whether `node` came back online since the last look (Veilid's
/// `is_online` swap), queueing every watch when it did.
pub(super) fn readiness(inner: &mut Inner, node: usize, now: u64) {
    let at = outbound::clock(inner, node, now);
    let on = watches::online(inner, node);
    let was = std::mem::replace(&mut inner.nodes[node].outbound.was_online, on);
    if on && !was {
        inspect_all(inner, node, at);
    }
}

/// Whether `node` has catch-up work a tick can make progress on now: a
/// readiness edge, an inspection due while it is online, or a re-hint
/// owed. A timer still running is not work (the pump ticks anyway).
pub(super) fn pending(inner: &Inner, node: usize, now: u64) -> bool {
    if outbound::asleep(inner, node) {
        return false;
    }
    let slot = &inner.nodes[node];
    let at = now.saturating_add_signed(slot.skew_ms.saturating_mul(1000));
    let on = watches::online(inner, node);
    let out = &slot.outbound;
    (on && out.queue.values().any(|t| *t <= at))
        || !out.rehints.is_empty()
        || (out.was_online != on && !slot.watches.is_empty())
}

/// Queue `node`'s due fallback inspections and run its due inspections;
/// each change found with its delay. An offline node runs none.
pub(super) fn run(
    inner: &mut Inner,
    latency: &Latency,
    node: usize,
    now: u64,
) -> Vec<(ValueChange, Duration)> {
    if !watches::online(inner, node) {
        return Vec::new();
    }
    let at = outbound::clock(inner, node, now);
    for key in outbound::fallbacks_due(inner, node, at) {
        enqueue_at(inner, node, &key, at);
    }
    let out = &mut inner.nodes[node].outbound;
    out.in_flight.retain(|_, done| *done > at);
    let free = MAX_IN_FLIGHT.saturating_sub(out.in_flight.len());
    let mut due: Vec<(u64, String)> = out
        .queue
        .iter()
        .filter(|(k, t)| **t <= at && !out.in_flight.contains_key(*k))
        .map(|(k, t)| (*t, k.clone()))
        .collect();
    due.sort();
    let mut found = Vec::new();
    for (_, key) in due.into_iter().take(free) {
        inner.nodes[node].outbound.queue.remove(&key);
        if !matches!(inner.nodes[node].watches.get(&key), Some(Some(_))) {
            continue;
        }
        match inspect(inner, latency, node, &key) {
            Some((change, delay)) => {
                inner.nodes[node]
                    .outbound
                    .in_flight
                    .insert(key, at.saturating_add(micros(delay)));
                if let Some(change) = change {
                    found.push((change, delay));
                }
            }
            None => requeue_failed(inner, node, &key, at),
        }
    }
    found
}

/// A failed inspection retries after `failure_backoff(5 s, 300 s, n)`.
fn requeue_failed(inner: &mut Inner, node: usize, key: &str, at: u64) {
    let failures = {
        let n = inner.nodes[node]
            .outbound
            .failures
            .entry(key.to_string())
            .or_insert(0);
        let attempt = *n;
        *n = n.saturating_add(1);
        attempt
    };
    let delay = micros(CHANGE_INSPECT_RETRY_INTERVAL)
        .saturating_mul(1u64 << failures.min(16))
        .min(micros(MAX_INSPECT_INTERVAL));
    enqueue_at(inner, node, key, at.saturating_add(delay));
}

/// One inspection of `node`'s watch on `key` (`watch_value.rs:1433-1520`):
/// `None` when no replica answered (a failure), else what it reports and
/// when its answer is in.
fn inspect(
    inner: &mut Inner,
    latency: &Latency,
    node: usize,
    key: &str,
) -> Option<(Option<ValueChange>, Duration)> {
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
    if reach.is_empty() {
        return None;
    }
    super::stores::touch(inner, key, &reach);
    let record = inner.records.get(key)?;
    let newer: BTreeSet<u32> = {
        let local = &inner.nodes[node].local;
        subkeys
            .into_iter()
            .filter(|s| {
                let held = local.get(&(key.to_string(), *s)).map(|v| v.seq);
                newest_seq(&record.replicas, &reach, *s as usize) > held
            })
            .collect()
    };
    outbound::inspected(inner, node, key, !newer.is_empty());
    let mut delay = latency.draw(node, latency::GET);
    let value = match newer.len() {
        0 => return Some((None, delay)),
        1 => {
            delay += latency.draw(node, latency::GET);
            let first = *newer.first()?;
            let Some(value) = read_one(inner, node, key, first, &reach) else {
                return Some((None, delay));
            };
            Some(value)
        }
        _ => None,
    };
    let desired = inner.nodes[node].watches.get_mut(key)?.as_mut()?;
    desired.count = desired.count.saturating_sub(1);
    let change = ValueChange {
        key: key.to_string(),
        subkeys: newer.into_iter().collect(),
        count: desired.count,
        value,
    };
    Some((Some(change), delay))
}

/// The one-subkey read of an inspection (`get_value(.., Newer)`): the
/// network's value stored locally when its seq moved, consuming the
/// subkey's report mark, and returned when newer than the local seq.
fn read_one(
    inner: &mut Inner,
    node: usize,
    key: &str,
    subkey: u32,
    reach: &[usize],
) -> Option<rekindle_records::port::RecordValue> {
    let held = inner.nodes[node]
        .local
        .get(&(key.to_string(), subkey))
        .cloned();
    let record = inner.records.get(key)?;
    let got = replicas::get(
        &record.schema,
        &record.replicas,
        reach,
        subkey as usize,
        held.as_ref(),
    );
    outbound::read_back(inner, node, key, subkey);
    let value = got?;
    if held.as_ref().is_none_or(|l| l.seq != value.seq) {
        inner.nodes[node]
            .local
            .insert((key.to_string(), subkey), value.clone());
    }
    held.is_none_or(|l| value.seq > l.seq).then_some(value)
}

impl Network {
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
