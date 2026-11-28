//! Watches on the simulated DHT (plan E2.B slice S5: M1, M8 and the death
//! shape), after the pinned veilid-core (`~/veilid-wt-058/veilid-core/src/
//! storage_manager`):
//!
//! - **the call** records the node's desired watch and answers whether a
//!   watch is wanted (`count` 0 cancels and answers `false`); a non-zero
//!   expiration sooner than the RPC timeout from now is `InvalidArgument`
//!   "expiration is too soon" (`watch_value.rs:290-363`). Nobody accepting
//!   it is not an error of the call: the outbound watch reconciles later
//!   (`outbound_watch_manager/outbound_watch.rs:258-311`);
//! - **placement:** the node's watch sits on up to [`GET_COUNT`] reachable
//!   replicas, its consensus count (`watch_value.rs:1334`), placed at the
//!   call and reconciled every [`WATCH_TICK`] (`CHECK_OUTBOUND_WATCHES_INTERVAL_SECS`,
//!   `constants.rs:16`). A replica refuses a watch past
//!   [`MEMBER_WATCH_LIMIT`] per member, or past [`PUBLIC_WATCH_LIMIT`]
//!   anonymous watches per record; a member is the owner or a SMPL member the
//!   node opened the record as (`record_store/record_store_inner/
//!   inbound_watches_interface.rs:76-141`, `veilid_config.rs:1019-1020`). A
//!   replica clamps the expiration to [`MAX_WATCH_EXPIRATION`] (none is the
//!   max) and refuses one sooner than [`MIN_WATCH_EXPIRATION`]
//!   (`record_store/inbound_watch.rs:70-89`, `storage_manager/mod.rs:441-447`);
//!   a reachable node renews its watches before they lapse, an unreachable
//!   one loses them;
//! - **a change:** a replica that stores a new value marks the subkey on
//!   every watch of it, the writer's own included: Veilid skips only a
//!   watch whose value-changed destination is the set's reply destination
//!   (0.5.8 `record_store_inner/inbound_watches_interface.rs:41,56-60`,
//!   `set_value.rs:917`), never the same target under safety routes;
//!   every [`WATCH_TICK`] (`SEND_VALUE_CHANGES_INTERVAL_SECS`, `constants.rs:14`)
//!   it sends each watch its changed subkeys, with the value only when one
//!   subkey changed, and counts the watch down (`inbound_watches_interface.rs:232-277`,
//!   `inbound_watch.rs:124-140`). An identical re-push stores nothing and
//!   marks nothing (`set_value.rs:823-842`);
//! - **the watcher** keeps a count per replica holding its watch: a push
//!   above it is invalid and dropped, and that replica's watch is cancelled
//!   at the next check; an equal one is a duplicate and goes on
//!   (`watch_value.rs:1890-1929`, [`super::outbound::per_node_push`]). The
//!   report's count stays one per watch (the netsim's stand-in for Veilid's
//!   remaining count). It drops the value when more than one subkey changed or the
//!   first it watches is not the first changed; drops a value not newer than
//!   its local copy and that subkey with it (nothing when none is left);
//!   stores a reported value before reporting it, else reports the range
//!   with no value, only its subkeys not already reported and unread
//!   (0.5.8 `watch_value.rs:2080-2263`, [`super::outbound`]'s report
//!   marks). A detached, offline or sleeping node, or one a partition cuts
//!   from the replica, receives nothing; a change
//!   goes over the watcher's value-changed route
//!   (`watch_value.rs:24`, `rpc_value_changed.rs`), is lost when that route
//!   died, and the watcher re-watches on the next check tick
//!   (`outbound_watch.rs:349-352`);
//! - **death:** a cancelled, closed (`close_record.rs:116-120`), expired or
//!   counted-out watch (`outbound_watch.rs:162-188`) ends at the next tick
//!   with empty subkeys, count 0 and no value (`watch_value.rs:781-787`).

use std::collections::BTreeSet;
use std::time::Duration;

use rekindle_records::port::{PortEvent, RecordSchema, ValueChange};

use super::replicas::GET_COUNT;
use super::{reachable, reaches};
use crate::network::{Inner, NetRecord, Network};

/// Anonymous watches per record per replica (`public_watch_limit`,
/// `veilid_config.rs:1019`).
pub(crate) const PUBLIC_WATCH_LIMIT: usize = 32;
/// Watches per member per record per replica (`member_watch_limit`,
/// `veilid_config.rs:1020`).
pub(crate) const MEMBER_WATCH_LIMIT: usize = 8;
/// `max_watch_expiration_ms` (`veilid_config.rs:1017`).
pub(crate) const MAX_WATCH_EXPIRATION: Duration = Duration::from_millis(600_000);
/// `rpc.timeout_ms` (`veilid_config.rs:944`, `storage_manager/mod.rs:446-447`).
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
    pub(crate) subkeys: BTreeSet<u32>,
    count: u32,
    expiration: u64,
    pub(crate) changed: BTreeSet<u32>,
    /// The watcher's value-changed route generation it was placed with.
    route: u64,
}

fn micros(d: Duration) -> u64 {
    u64::try_from(d.as_micros()).unwrap_or(u64::MAX)
}

/// The too-soon check of the call (0.5.8 `watch_value.rs:336-343`).
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
            // A change of parameters is inspected (0.5.8 `watch_value.rs:1317-1322`).
            let changed = watches.get(key).and_then(Option::as_ref).is_none_or(|old| {
                (&old.subkeys, old.expiration, old.member) != (&d.subkeys, d.expiration, d.member)
            });
            watches.insert(key.to_string(), Some(d));
            if changed {
                super::outbound::params_changed(inner, node, key);
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
    let (mut held, mut added, mut accepted, mut refused) = (0, false, Vec::new(), 0u64);
    for r in reachable(&record.replicas, &inner.faults, node) {
        let watches = &mut record.replicas[r].watches;
        if let Some(w) = watches.iter_mut().find(|w| w.node == node) {
            if w.member == d.member {
                (w.subkeys, w.count, w.expiration) = (d.subkeys.clone(), d.count, expiration);
                held += 1;
                accepted.push(r);
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
            accepted.push(r);
        } else {
            // `InboundWatchValueResult::Rejected` (`:120-131`).
            refused += 1;
        }
    }
    for r in accepted {
        super::outbound::per_node_placed(inner, node, key, r, d.count);
    }
    inner.watch_refusals += refused;
    // A replica newly holding the watch is inspected (0.5.8 `watch_value.rs:1317-1322`).
    if added {
        super::outbound::established(inner, node, key, now);
        super::catchup::enqueue(inner, node, key, now);
    }
}

/// The replicas at `stored` took a new value at `subkey`: mark it on every
/// watch of it, the writer's included.
pub(crate) fn mark(record: &mut NetRecord, stored: &[usize], subkey: u32) {
    for r in stored {
        if let Some(replica) = record.replicas.get_mut(*r) {
            for w in &mut replica.watches {
                if w.subkeys.contains(&subkey) {
                    w.changed.insert(subkey);
                }
            }
        }
    }
}

/// How many replicas hold `node`'s watch on `key` (Veilid's per-node
/// watches of the outbound watch).
pub(crate) fn held(inner: &Inner, node: usize, key: &str) -> usize {
    inner.records.get(key).map_or(0, |r| {
        r.replicas
            .iter()
            .filter(|x| x.watches.iter().any(|w| w.node == node))
            .count()
    })
}

/// The last change of a dead watch (`watch_value.rs:781-787`).
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
    super::outbound::per_node_ended(inner, node, key);
}

/// The node's finished per-node watches on `key` (count 0, a counted-out
/// or invalid one) leave its state, and the replica's watch with them
/// (Veilid's background process cancels it, `watch_value.rs:1903-1905`,
/// `outbound_watch_manager/mod.rs:322-357`); the check re-places after.
fn drop_finished(inner: &mut Inner, node: usize, key: &str) {
    let finished = super::outbound::take_finished(inner, node, key);
    if let Some(record) = inner.records.get_mut(key) {
        for r in finished {
            if let Some(replica) = record.replicas.get_mut(r) {
                replica.watches.retain(|w| w.node != node);
            }
        }
    }
}

/// After the tick's sends, an online, awake watcher whose value-changed
/// route died re-watches with its new one (`outbound_watch.rs:349-352`);
/// changes sent before then were lost, so its fallback timer resets
/// ([`super::outbound::route_lost`]).
fn renew_routes(inner: &mut Inner, now: u64) {
    let epochs: Vec<(u64, bool)> = (0..inner.nodes.len())
        .map(|n| {
            let live = online(inner, n) && !super::outbound::asleep(inner, n);
            (inner.nodes[n].route_epoch, live)
        })
        .collect();
    let mut lost = BTreeSet::new();
    for (key, record) in &mut inner.records {
        for replica in &mut record.replicas {
            for w in &mut replica.watches {
                if let Some((epoch, true)) = epochs.get(w.node).copied() {
                    if w.route != epoch {
                        lost.insert((w.node, key.clone()));
                    }
                    w.route = epoch;
                }
            }
        }
    }
    for (node, key) in lost {
        super::outbound::route_lost(inner, node, &key, now);
    }
}

/// An event the tick sends once the lock is released.
type Out = (usize, usize, &'static str, PortEvent);

/// The watcher's side of one change from replica index `replica` (0.5.8
/// `watch_value.rs:1819-1939,2080-2262`).
pub(super) fn receive(
    inner: &mut Inner,
    node: usize,
    key: &str,
    replica: usize,
    (subkeys, count, mut value): (BTreeSet<u32>, u32, Option<super::replicas::Cipher>),
    now: u64,
) -> Option<ValueChange> {
    if !matches!(inner.nodes[node].watches.get(key), Some(Some(_))) {
        return None;
    }
    // The per-node count first: a count above it drops the change before
    // the silence timer is touched (`watch_value.rs:1890-1936`).
    if !super::outbound::per_node_push(inner, node, key, replica, count) {
        return None;
    }
    // A valid push resets the silence timer (`watch_value.rs:1931-1936`).
    super::outbound::pushed(inner, node, key, now);
    // A watched value past the size rule makes the change invalid
    // (`watch_value.rs:2188-2200`): it is dropped.
    if let (Some(v), Some(record)) = (&value, inner.records.get(key)) {
        if !super::size::fits(&record.schema, &v.value) {
            return None;
        }
    }
    let desired = inner.nodes[node].watches.get_mut(key)?.as_mut()?;
    let watched = desired.subkeys.clone();
    // The report carries the replica's count; one count per watch (Veilid
    // keeps a remaining count beside the per-node ones, `:1959-1968`).
    desired.count = count;
    let first = *subkeys.first()?;
    if subkeys.len() > 1 {
        value = None;
        *inner.nodes[node]
            .outbound
            .coalesced
            .entry(key.to_string())
            .or_default() += 1;
    }
    let mut reportable: BTreeSet<u32> = subkeys.intersection(&watched).copied().collect();
    if reportable.first() != Some(&first) {
        value = None;
    }
    if let Some(v) = &value {
        if inner.nodes[node]
            .local
            .get(&(key.to_string(), first))
            .is_some_and(|l| v.value.seq <= l.seq)
        {
            value = None;
            reportable.pop_first();
            if reportable.is_empty() {
                return None;
            }
        }
    }
    if let Some(v) = &value {
        // The pushed bytes are the replica's ciphertext, nonce and all.
        super::replicas::keep(inner, node, key, first, v);
        super::outbound::value_reported(inner, node, key, &reportable);
    } else {
        reportable = super::outbound::hinted(inner, node, key, &reportable);
        if reportable.is_empty() {
            return None;
        }
    }
    Some(ValueChange {
        key: key.to_string(),
        subkeys: reportable.into_iter().collect(),
        count,
        value: value.map(|v| v.value),
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
            super::outbound::ended(&mut inner, node, key);
        }
        self.send(node, node, "WatchDeath", death(key), false);
    }

    /// Whether a tick has work: a change to send, a watch to end or expire.
    pub(crate) fn watch_work(inner: &Inner, now: u64) -> bool {
        inner.nodes.iter().enumerate().any(|(i, n)| {
            super::catchup::pending(inner, i, now)
                || super::rehydrate::pending(inner, i)
                || (online(inner, i)
                    && n.watches
                        .values()
                        .any(|d| d.as_ref().is_none_or(|d| d.expiration.is_some())))
        }) || inner.records.values().any(|r| {
            r.replicas
                .iter()
                .any(|x| x.watches.iter().any(|w| !w.changed.is_empty()))
        })
    }

    /// The outbound watch check of every node (see the module notes), in
    /// key order for the seed: Veilid's `check_outbound_watches_task`, which
    /// runs only while the node's DHT is online (0.5.8
    /// `storage_manager/tasks/mod.rs:102,110,140-144`), so an offline node's
    /// watches neither die nor renew nor re-hint until it is back. The
    /// readiness edge is the node's own, not the task's: it is seen offline too.
    pub(crate) fn tick_watches(&self) {
        let now = self.now_us();
        let mut out: Vec<Out> = Vec::new();
        let mut inspected = Vec::new();
        {
            let mut guard = self.inner.lock();
            let inner = &mut *guard;
            for node in 0..inner.nodes.len() {
                // A suspended process runs no tick ([`crate::Fault::Sleep`]).
                if super::outbound::asleep(inner, node) {
                    continue;
                }
                super::catchup::readiness(inner, node, now);
                if !online(inner, node) {
                    continue;
                }
                let mut keys: Vec<String> = inner.nodes[node].watches.keys().cloned().collect();
                keys.sort();
                for key in keys {
                    let watches = &mut inner.nodes[node].watches;
                    if let Some(Some(d)) = watches.get(&key) {
                        if d.expiration.is_some_and(|e| e <= now) || d.count == 0 {
                            watches.insert(key.clone(), None);
                        }
                    }
                    drop_finished(inner, node, &key);
                    let watches = &mut inner.nodes[node].watches;
                    if matches!(watches.get(&key), Some(None)) {
                        watches.remove(&key);
                        drop_replica_watches(inner, node, &key);
                        super::outbound::ended(inner, node, &key);
                        out.push((node, node, "WatchDeath", death(&key)));
                    } else {
                        place(inner, node, &key, now);
                    }
                }
                for (change, delay) in super::catchup::run(inner, &self.latency, node, now) {
                    inspected.push((node, change, delay));
                }
                for change in super::outbound::take_rehints(inner, node) {
                    out.push((node, node, "Rehint", PortEvent::ValueChange(change)));
                }
            }
        }
        self.send_out(out);
        self.send_inspected(inspected);
    }

    /// Every replica sends its watches' changes (Veilid's
    /// `send_value_changes_task`, last in the storage tick, 0.5.8
    /// `storage_manager/tasks/mod.rs:155-156`), then the watchers renew
    /// their dead value-changed routes.
    pub(crate) fn tick_value_changes(&self) {
        let now = self.now_us();
        let mut out: Vec<Out> = Vec::new();
        {
            let mut guard = self.inner.lock();
            let inner = &mut *guard;
            let mut keys: Vec<String> = inner.records.keys().cloned().collect();
            keys.sort();
            for key in keys {
                let mut sent = Vec::new();
                if let Some(record) = inner.records.get_mut(&key) {
                    for (r, replica) in record.replicas.iter_mut().enumerate() {
                        let home = replica.home;
                        let ciphers: Vec<Option<super::replicas::Cipher>> =
                            (0..replica.values.len())
                                .map(|s| replica.cipher(s))
                                .collect();
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
                                (1, Some(s)) => ciphers.get(*s as usize).cloned().flatten(),
                                _ => None,
                            };
                            sent.push((home, r, w.node, w.route, (subkeys, w.count, value)));
                            w.count > 0
                        });
                    }
                }
                for (home, r, node, route, change) in sent {
                    if !online(inner, node)
                        || super::outbound::asleep(inner, node)
                        || !reaches(&inner.faults, node, home)
                        || inner
                            .faults
                            .drop_value_changes
                            .contains(&(node, key.clone()))
                    {
                        continue;
                    }
                    // Sent over a dead value-changed route: lost.
                    if route != inner.nodes[node].route_epoch {
                        continue;
                    }
                    if let Some(change) = receive(inner, node, &key, r, change, now) {
                        out.push((home, node, "ValueChange", PortEvent::ValueChange(change)));
                    }
                }
            }
            renew_routes(inner, now);
        }
        self.send_out(out);
    }

    /// Send what a tick queued, once the lock is released.
    fn send_out(&self, out: Vec<Out>) {
        for (from, to, label, event) in out {
            // A change is an RPC to the watcher; a death is the node's own.
            let delay = if from == to {
                Duration::ZERO
            } else {
                self.latency.draw(to, crate::latency::MESSAGE)
            };
            self.send_after(from, to, label, event, false, delay);
        }
    }
}
