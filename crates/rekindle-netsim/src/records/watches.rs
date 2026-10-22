//! Watches on the simulated DHT (plan E2.B slice S5: M1, M8 and the death
//! shape), after the pinned veilid-core (`~/veilid/veilid-core/src/
//! storage_manager`):
//!
//! - **the call** records the node's desired watch and answers whether a
//!   watch is wanted (`count` 0 cancels and answers `false`); a non-zero
//!   expiration sooner than the RPC timeout from now is `InvalidArgument`
//!   "expiration is too soon" (`watch_value.rs:224-300`). Nobody accepting
//!   it is not an error of the call: the outbound watch reconciles later
//!   (`outbound_watch_manager/outbound_watch.rs:185-215`);
//! - **placement:** the node's watch sits on up to [`GET_COUNT`] reachable
//!   replicas, its consensus count (`watch_value.rs:1185`), placed at the
//!   call and reconciled every [`WATCH_TICK`] (`CHECK_OUTBOUND_WATCHES_INTERVAL_SECS`,
//!   `constants.rs:16`). A replica refuses a watch past
//!   [`MEMBER_WATCH_LIMIT`] per member, or past [`PUBLIC_WATCH_LIMIT`]
//!   anonymous watches per record; a member is the owner or a SMPL member the
//!   node opened the record as (`record_store/record_store_inner/
//!   inbound_watches_interface.rs:75-138`, `veilid_config.rs:921-922`). A
//!   replica clamps the expiration to [`MAX_WATCH_EXPIRATION`] (none is the
//!   max) and refuses one sooner than [`MIN_WATCH_EXPIRATION`]
//!   (`record_store/inbound_watch.rs:70-89`, `storage_manager/mod.rs:361-367`);
//!   a reachable node renews its watches before they lapse, an unreachable
//!   one loses them;
//! - **a change:** a replica that stores a new value marks the subkey on
//!   every watch of it but the writer's (`inbound_watches_interface.rs:32-69`);
//!   every [`WATCH_TICK`] (`SEND_VALUE_CHANGES_INTERVAL_SECS`, `constants.rs:14`)
//!   it sends each watch its changed subkeys, with the value only when one
//!   subkey changed, and counts the watch down (`:228-273`,
//!   `inbound_watch.rs:124-140`). An identical re-push stores nothing and
//!   marks nothing (`set_value.rs:700-719`);
//! - **the watcher** drops the value when more than one subkey changed or the
//!   first it watches is not the first changed; drops a value not newer than
//!   its local copy and that subkey with it (nothing when none is left);
//!   stores a reported value before reporting it, else reports the range
//!   with no value (`watch_value.rs:1866-1998`). A detached or offline node,
//!   or one a partition cuts from the replica, receives nothing; a change
//!   goes over the watcher's value-changed route
//!   (`watch_value.rs:23`, `rpc_value_changed.rs`), is lost when that route
//!   died, and the watcher re-watches on the next check tick
//!   (`outbound_watch.rs:272-275`);
//! - **death:** a cancelled, closed (`close_record.rs:116-120`), expired or
//!   counted-out watch (`outbound_watch.rs:89-115`) ends at the next tick
//!   with empty subkeys, count 0 and no value (`watch_value.rs:697-703`).

use std::collections::BTreeSet;
use std::time::Duration;

use rekindle_records::port::{PortEvent, RecordSchema, RecordValue, ValueChange};

use super::replicas::GET_COUNT;
use super::{reachable, reaches};
use crate::network::{Inner, NetRecord, Network};

/// Anonymous watches per record per replica (`public_watch_limit`,
/// `veilid_config.rs:921`).
pub(crate) const PUBLIC_WATCH_LIMIT: usize = 32;
/// Watches per member per record per replica (`member_watch_limit`,
/// `veilid_config.rs:922`).
pub(crate) const MEMBER_WATCH_LIMIT: usize = 8;
/// `max_watch_expiration_ms` (`veilid_config.rs:920`).
pub(crate) const MAX_WATCH_EXPIRATION: Duration = Duration::from_millis(600_000);
/// `rpc.timeout_ms` (`veilid_config.rs:852`, `storage_manager/mod.rs:366-367`).
pub(crate) const MIN_WATCH_EXPIRATION: Duration = Duration::from_millis(5_000);
/// The send-changes and check-watches tick (`constants.rs:14,16`).
pub(crate) const WATCH_TICK: Duration = Duration::from_secs(1);

/// A node's desired watch (Veilid's outbound watch's desired parameters).
#[derive(Debug, Clone)]
pub(crate) struct Desired {
    pub subkeys: BTreeSet<u32>,
    /// Absolute simulated µs; `None` renews forever.
    pub expiration: Option<u64>,
    pub count: u32,
    /// The member the node opened the record as, if any.
    pub member: Option<[u8; 32]>,
}

/// A watch a replica holds (Veilid's inbound watch).
#[derive(Debug, Clone)]
pub(crate) struct ReplicaWatch {
    pub node: usize,
    member: Option<[u8; 32]>,
    subkeys: BTreeSet<u32>,
    count: u32,
    expiration: u64,
    changed: BTreeSet<u32>,
    /// The watcher's value-changed route generation it was placed with.
    route: u64,
}

fn micros(d: Duration) -> u64 {
    u64::try_from(d.as_micros()).unwrap_or(u64::MAX)
}

/// The too-soon check of the call (`watch_value.rs:269-278`).
pub(crate) fn too_soon(expiration_us: Option<u64>, now: u64) -> bool {
    expiration_us.is_some_and(|e| e != 0 && e < now.saturating_add(micros(MIN_WATCH_EXPIRATION)))
}

/// Whether `public` is the record's owner or a SMPL member.
fn is_member(record: &NetRecord, public: &[u8; 32]) -> bool {
    record.owner == *public
        || matches!(&record.schema, RecordSchema::Smpl { members, .. }
            if members.iter().any(|m| m.key == *public))
}

/// The member `writer` (an open's keypair) makes the node, if any.
pub(crate) fn member_of(record: &NetRecord, writer: Option<&str>) -> Option<[u8; 32]> {
    let public = super::writer_public(writer?).ok()?;
    is_member(record, &public).then_some(public)
}

pub(super) fn online(inner: &Inner, node: usize) -> bool {
    !inner.faults.detached.contains(&node)
        && !inner.faults.dht_offline.contains(&node)
        && !inner.faults.not_ready.contains(&node)
}

/// Set `node`'s desired watch on `key` (`None` cancels; it dies at the next
/// tick) and place it now when the node is online.
pub(crate) fn desire(
    inner: &mut Inner,
    node: usize,
    key: &str,
    desired: Option<Desired>,
    now: u64,
) {
    let watches = &mut inner.nodes[node].watches;
    match desired {
        Some(d) => {
            // A change of parameters is inspected (`watch_value.rs:1166-1171`).
            let changed = watches.get(key).and_then(Option::as_ref).is_none_or(|old| {
                (&old.subkeys, old.expiration, old.member) != (&d.subkeys, d.expiration, d.member)
            });
            watches.insert(key.to_string(), Some(d));
            if changed {
                super::catchup::enqueue(inner, node, key, now);
            }
            place(inner, node, key, now);
        }
        None => {
            if let Some(w) = watches.get_mut(key) {
                *w = None;
            }
        }
    }
}

/// Renew `node`'s watch where it is held and the replica is reachable, and
/// place it on more reachable replicas until [`GET_COUNT`] hold it.
fn place(inner: &mut Inner, node: usize, key: &str, now: u64) {
    if !online(inner, node) {
        return;
    }
    let Some(Some(d)) = inner.nodes[node].watches.get(key).cloned() else {
        return;
    };
    let max = now.saturating_add(micros(MAX_WATCH_EXPIRATION));
    let expiration = match d.expiration {
        None => max,
        Some(e) if e > max => max,
        Some(e) if e < now.saturating_add(micros(MIN_WATCH_EXPIRATION)) => return,
        Some(e) => e,
    };
    let epoch = inner.nodes[node].route_epoch;
    let Some(record) = inner.records.get_mut(key) else {
        return;
    };
    let (mut held, mut added) = (0, false);
    for r in reachable(&record.replicas, &inner.faults.partitions, node) {
        let watches = &mut record.replicas[r].watches;
        if let Some(w) = watches.iter_mut().find(|w| w.node == node) {
            if w.member == d.member {
                (w.subkeys, w.count, w.expiration) = (d.subkeys.clone(), d.count, expiration);
                held += 1;
            }
            continue;
        }
        if held >= GET_COUNT {
            continue;
        }
        let (taken, limit) = match d.member {
            Some(m) => (
                watches.iter().filter(|w| w.member == Some(m)).count(),
                MEMBER_WATCH_LIMIT,
            ),
            None => (
                watches.iter().filter(|w| w.member.is_none()).count(),
                PUBLIC_WATCH_LIMIT,
            ),
        };
        if taken < limit {
            watches.push(ReplicaWatch {
                node,
                member: d.member,
                subkeys: d.subkeys.clone(),
                count: d.count,
                expiration,
                changed: BTreeSet::new(),
                route: epoch,
            });
            held += 1;
            added = true;
        }
    }
    // A replica newly holding the watch is inspected (`watch_value.rs:1166-1171`).
    if added {
        super::catchup::enqueue(inner, node, key, now);
    }
}

/// The replicas at `stored` took a new value at `subkey`: mark it on their
/// watches, all but `writer`'s.
pub(crate) fn mark(record: &mut NetRecord, stored: &[usize], subkey: u32, writer: usize) {
    for r in stored {
        if let Some(replica) = record.replicas.get_mut(*r) {
            for w in &mut replica.watches {
                if w.node != writer && w.subkeys.contains(&subkey) {
                    w.changed.insert(subkey);
                }
            }
        }
    }
}

/// How many replicas hold `node`'s watch on `key`.
#[cfg(test)]
pub(crate) fn held(inner: &Inner, node: usize, key: &str) -> usize {
    inner.records.get(key).map_or(0, |r| {
        r.replicas
            .iter()
            .filter(|x| x.watches.iter().any(|w| w.node == node))
            .count()
    })
}

/// The last change of a dead watch (`watch_value.rs:697-703`).
pub(crate) fn death(key: &str) -> PortEvent {
    PortEvent::ValueChange(ValueChange {
        key: key.to_string(),
        subkeys: Vec::new(),
        count: 0,
        value: None,
    })
}

fn drop_replica_watches(inner: &mut Inner, node: usize, key: &str) {
    if let Some(record) = inner.records.get_mut(key) {
        for replica in &mut record.replicas {
            replica.watches.retain(|w| w.node != node);
        }
    }
}

/// After the tick's sends, an online watcher whose value-changed route died
/// re-watches with its new one (`outbound_watch.rs:272-275`); changes sent
/// before then were lost.
fn renew_routes(inner: &mut Inner) {
    let epochs: Vec<(u64, bool)> = (0..inner.nodes.len())
        .map(|n| (inner.nodes[n].route_epoch, online(inner, n)))
        .collect();
    for record in inner.records.values_mut() {
        for replica in &mut record.replicas {
            for w in &mut replica.watches {
                if let Some((epoch, true)) = epochs.get(w.node).copied() {
                    w.route = epoch;
                }
            }
        }
    }
}

/// An event the tick sends once the lock is released.
type Out = (usize, usize, &'static str, PortEvent);

/// The watcher's side of one change (`watch_value.rs:1866-1998`).
pub(super) fn receive(
    inner: &mut Inner,
    node: usize,
    key: &str,
    (subkeys, count, mut value): (BTreeSet<u32>, u32, Option<RecordValue>),
) -> Option<ValueChange> {
    // A watched value past the size rule makes the change invalid
    // (`watch_value.rs:1934-1946`): it is dropped.
    if let (Some(v), Some(record)) = (&value, inner.records.get(key)) {
        if !super::size::fits(&record.schema, v) {
            return None;
        }
    }
    let desired = inner.nodes[node].watches.get_mut(key)?.as_mut()?;
    let watched = desired.subkeys.clone();
    // The per-node state takes the replica's count (`watch_value.rs:1882-1891`).
    desired.count = count;
    let first = *subkeys.first()?;
    if subkeys.len() > 1 {
        value = None;
    }
    let mut reportable: BTreeSet<u32> = subkeys.intersection(&watched).copied().collect();
    if reportable.first() != Some(&first) {
        value = None;
    }
    let local = &mut inner.nodes[node].local;
    if let Some(v) = &value {
        if local
            .get(&(key.to_string(), first))
            .is_some_and(|l| v.seq <= l.seq)
        {
            value = None;
            reportable.pop_first();
            if reportable.is_empty() {
                return None;
            }
        }
    }
    if let Some(v) = &value {
        local.insert((key.to_string(), first), v.clone());
    } else if reportable.is_empty() {
        return None;
    }
    Some(ValueChange {
        key: key.to_string(),
        subkeys: reportable.into_iter().collect(),
        count,
        value,
    })
}

impl Network {
    /// The simulated time in Veilid's form (µs since the Unix epoch).
    pub(crate) fn now_us(&self) -> u64 {
        crate::clock::SIM_EPOCH_MS
            .saturating_mul(1000)
            .saturating_add(micros(self.epoch.elapsed()))
    }

    /// What `node`'s own clock reads (µs): true time plus its skew. A
    /// watch's expiration is absolute: the node checks it on this clock,
    /// each replica on its own (true time, [`place`]).
    pub(crate) fn node_now_us(&self, node: usize) -> u64 {
        let skew = self.inner.lock().nodes.get(node).map_or(0, |s| s.skew_ms);
        self.now_us()
            .saturating_add_signed(skew.saturating_mul(1000))
    }

    /// The `WatchDeath` fault: the node's watch on `key` dies now.
    pub(crate) fn kill_watch(&self, node: usize, key: &str) {
        {
            let mut inner = self.inner.lock();
            if inner
                .nodes
                .get_mut(node)
                .and_then(|s| s.watches.remove(key))
                .is_none()
            {
                return;
            }
            drop_replica_watches(&mut inner, node, key);
        }
        self.send(node, node, "WatchDeath", death(key), false);
    }

    /// Whether a tick has work: a change to send, a watch to end or expire.
    pub(crate) fn watch_work(inner: &Inner) -> bool {
        inner.nodes.iter().enumerate().any(|(i, n)| {
            super::catchup::pending(inner, i)
                || super::rehydrate::pending(inner, i)
                || n.watches
                    .values()
                    .any(|d| d.as_ref().is_none_or(|d| d.expiration.is_some()))
        }) || inner.records.values().any(|r| {
            r.replicas
                .iter()
                .any(|x| x.watches.iter().any(|w| !w.changed.is_empty()))
        })
    }

    /// One watch tick (see the module notes), in key order for the seed.
    pub(crate) fn tick_watches(&self) {
        let now = self.now_us();
        let mut out: Vec<Out> = Vec::new();
        let mut inspected = Vec::new();
        {
            let mut guard = self.inner.lock();
            let inner = &mut *guard;
            for node in 0..inner.nodes.len() {
                super::catchup::readiness(inner, node, now);
                let mut keys: Vec<String> = inner.nodes[node].watches.keys().cloned().collect();
                keys.sort();
                for key in keys {
                    let watches = &mut inner.nodes[node].watches;
                    if let Some(Some(d)) = watches.get(&key) {
                        if d.expiration.is_some_and(|e| e <= now) || d.count == 0 {
                            watches.insert(key.clone(), None);
                        }
                    }
                    if matches!(watches.get(&key), Some(None)) {
                        watches.remove(&key);
                        drop_replica_watches(inner, node, &key);
                        out.push((node, node, "WatchDeath", death(&key)));
                    } else {
                        place(inner, node, &key, now);
                    }
                }
                for (change, delay) in super::catchup::run(inner, &self.latency, node, now) {
                    inspected.push((node, change, delay));
                }
            }
            let mut keys: Vec<String> = inner.records.keys().cloned().collect();
            keys.sort();
            for key in keys {
                let mut sent = Vec::new();
                if let Some(record) = inner.records.get_mut(&key) {
                    for replica in &mut record.replicas {
                        let (home, values) = (replica.home, &replica.values);
                        replica.watches.retain_mut(|w| {
                            if w.expiration <= now {
                                return false;
                            }
                            if w.changed.is_empty() {
                                return true;
                            }
                            let subkeys = std::mem::take(&mut w.changed);
                            w.count = w.count.saturating_sub(1);
                            let value = match (subkeys.len(), subkeys.first()) {
                                (1, Some(s)) => values.get(*s as usize).cloned().flatten(),
                                _ => None,
                            };
                            sent.push((home, w.node, w.route, (subkeys, w.count, value)));
                            w.count > 0
                        });
                    }
                }
                for (home, node, route, change) in sent {
                    if !online(inner, node) || !reaches(&inner.faults.partitions, node, home) {
                        continue;
                    }
                    // Sent over a dead value-changed route: lost.
                    if route != inner.nodes[node].route_epoch {
                        continue;
                    }
                    if let Some(change) = receive(inner, node, &key, change) {
                        out.push((home, node, "ValueChange", PortEvent::ValueChange(change)));
                    }
                }
            }
            renew_routes(inner);
        }
        for (from, to, label, event) in out {
            // A change is an RPC to the watcher; a death is the node's own.
            let delay = if from == to {
                Duration::ZERO
            } else {
                self.latency.draw(to, crate::latency::MESSAGE)
            };
            self.send_after(from, to, label, event, false, delay);
        }
        self.send_inspected(inspected);
    }
}
