//! The faults a scenario puts in force (plan C9.3) and how the sim
//! applies them; split from `lib.rs` along its seam.

use rekindle_records::port::RecordSchema;

use super::{pair, toggle, Netsim, PortEvent};
use crate::network::{Faults, NetRecord};
use crate::records::GET_COUNT;

/// A fault to put in force (plan C9.3).
#[derive(Debug, Clone)]
pub enum Fault {
    /// The node's next `calls` DHT calls answer "try again".
    TryAgain { node: usize, calls: u32 },
    /// Every DHT call of the node fails (or stops failing): no connection.
    DhtOffline { node: usize, offline: bool },
    /// Every set by the node that `of` selects lands below consensus (no
    /// replica accepts it and nothing is stored, as a Veilid `set_value`
    /// can), until [`Fault::Heal`]; its other writes are unaffected.
    WriteFails { node: usize, of: Writes },
    /// Every set by the node that `of` selects reaches only the
    /// `accepting` replicas closest to the record, which store it, until
    /// [`Fault::Heal`]. `0 < accepting < get_value_count`: Veilid counts a
    /// set that times out or exhausts with fewer than `get_value_count`
    /// `consensus_nodes` as offline (`storage_manager/mod.rs:781-811`,
    /// `set_value.rs:596-632`), and one with more as landed, so only
    /// fewer is below consensus. With
    /// `AllowOffline(false)` the writer stores and queues nothing locally
    /// (`local_record_store_interface.rs:60-62`) and hears `TryAgain`
    /// (0.5.8 `set_value.rs:731-738`), never `Ok(None)`, while a reader whose fanout reaches an accepting replica sees the
    /// value.
    WritePartial {
        node: usize,
        of: Writes,
        accepting: usize,
    },
    /// The two groups cannot exchange app messages or calls, and neither
    /// reaches the record replicas placed beside the other's nodes (a
    /// stale read or a set below consensus follows from which replicas a
    /// side reaches; `records` notes the placement).
    Partition { a: Vec<usize>, b: Vec<usize> },
    /// Every partition and offline DHT ends, and every attached node whose
    /// public internet was not ready is ready again; the record pools' own
    /// timers re-push their held writes.
    Heal,
    /// This fraction (per mille) of app messages and calls is dropped.
    DropMessages { per_mille: u32 },
    /// This fraction (per mille) of the app messages and calls the node
    /// sends, and of the answers to calls it receives, is dropped, while
    /// what is sent to it still arrives: a one-direction loss (plan C9.4,
    /// findings 33, 45 and 62). The DHT is a separate path: its sets, gets,
    /// inspects and watches, and its replicas' answers, are never dropped
    /// (LV17 follow-up ii). `0` ends it.
    DropFrom { node: usize, per_mille: u32 },
    /// The node never answers app calls; its callers time out.
    Unanswered { node: usize, unanswered: bool },
    /// A call to the node's routes never resolves: no answer and no
    /// Veilid `Timeout`, as a call stuck before Veilid's answer wait
    /// (route resolution, rendering, the first hop's send, none bounded:
    /// `rpc_processor/mod.rs:1718-1830`; live finding 78). The question is
    /// lost; only the caller's own deadline ends it.
    CallHangs { node: usize, hangs: bool },
    /// An app message to the node's routes is never dispatched: the send
    /// neither returns nor fails, as a send stuck in the stages before the
    /// first hop takes it (destination, render and safety-route compile,
    /// `send_envelope`, none bounded: `rpc_processor/mod.rs:1882-1926`).
    /// The message is lost; only the sender's own deadline ends the send.
    SendHangs { node: usize, hangs: bool },
    /// The node's clock runs this far from simulated true time.
    ClockSkew { node: usize, ms: i64 },
    /// The node's watch on `key` dies: Veilid sends a last change with
    /// empty subkeys, `count == 0` and no value (`watch_value.rs:697-703`).
    WatchDeath { node: usize, key: String },
    /// The node's own routes die: the owner hears of it, its importers do
    /// not (their sends are lost until they fetch the new route).
    RouteDeath { node: usize },
    /// The first hop of the node's safety route fails on its next app
    /// message or call: that send fails `NoConnection`, which our port
    /// reports as a `Send` error (our side: the peer's route is kept,
    /// `rekindle-protocol` `ports.rs` `peer_route_unusable`, plan VU5).
    /// Veilid 0.5.8 counts it as a send failure, which
    /// only recompiles the route (`route_stats.rs:19-35`: not
    /// `means_route_dead`, but `needs_recompile`, `route_compile.rs:672`):
    /// the route lives, nobody hears `dead_routes`, and the next send goes
    /// out on the recompiled route ([`routes`](crate::routes)).
    RouteLinkFailure { node: usize },
    /// The node's own route of `class` stops carrying traffic end to end:
    /// what is sent over it or to it is lost, its loopback tests fail, and
    /// a run of 3 (Ordered) or 5 (Unordered) kills it, reported to the
    /// owner as `dead_routes` ([`crate::loopback`]).
    LoopbackFailures {
        node: usize,
        class: rekindle_records::port::RouteClass,
    },
    /// The node's network blocks UDP (or stops blocking it): it cannot
    /// allocate an Unordered route (no fallback to Ordered,
    /// [`routes`](crate::routes)), and its live Unordered routes fail end
    /// to end as [`Fault::LoopbackFailures`] until UDP returns or they die.
    UdpBlocked { node: usize, blocked: bool },
    /// The node's process stalled `ms` (a suspended or starved executor).
    /// From 2.5 s on, Veilid 0.5.8's attachment manager sends the
    /// unchanged attachment state again ([`attach`](crate::attach)).
    Stall { node: usize, ms: u64 },
    /// The node detaches (or reattaches): its DHT calls fail, it neither
    /// sends nor receives app messages or calls (a call to it times out),
    /// no watch change reaches it, and it hears Detaching then Detached
    /// (or Attaching up to AttachedGood); its ready watch follows
    /// ([`attach`]).
    Detach { node: usize, detached: bool },
    /// The node's public internet turns unready (or ready again) while it
    /// stays attached, as on a `RoutingDomainReadyEvent`: its DHT calls
    /// answer "try again", its ready watch follows, and it hears an
    /// attachment update with `is_attached` still true.
    InternetReady { node: usize, ready: bool },
    /// The node's process is suspended (a desktop sleep) or resumes: while
    /// asleep its storage manager's tick does not run and nothing sent to
    /// it arrives (a value-changed push, an app message or call), and it
    /// stays attached, with no Offline transition. Veilid 0.5.8 has no
    /// tick-lag sweep (0.5.7's `CHANGE_INSPECT_LAG` is gone from
    /// `storage_manager/constants.rs`): a missed push surfaces through the
    /// fallback timer, on the node's wall clock
    /// (`records/outbound.rs`). Pair it with [`Fault::ClockSkew`] for the
    /// wall clock jump of a sleep the monotonic clock did not see.
    Sleep { node: usize, asleep: bool },
    /// Nodes join closer to `key`: `count` of its replicas are pushed out
    /// for empty stores, their watches gone (`records/outside.rs`).
    ReplicaChurn { key: String, count: usize },
}

/// Which of a node's sets [`Fault::WriteFails`] or [`Fault::WritePartial`]
/// refuses: the network refusing that writer's sets, chosen by what they
/// write.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Writes {
    /// Every set of this record.
    Record(String),
    /// Every set of this subkey in the records `record` selects (any
    /// record when `None`): a record the node creates mid-scenario (a
    /// joiner's member record) is covered before its key is known.
    Subkey {
        subkey: u32,
        record: Option<Records>,
    },
}

/// Which records a [`Writes::Subkey`] covers.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Records {
    /// This record only.
    Key(String),
    /// The records the writing node created with this many owner subkeys
    /// and no members (`DFLT(n)`): a joiner's own member record is
    /// `DFLT(8)`, and the channel, presence and other records it writes
    /// are not.
    CreatedDflt(u16),
}

impl Writes {
    pub(crate) fn selects(
        &self,
        node: usize,
        key: &str,
        subkey: u32,
        record: Option<&NetRecord>,
    ) -> bool {
        match self {
            Self::Record(selected) => selected == key,
            Self::Subkey {
                subkey: selected,
                record: of,
            } => {
                *selected == subkey
                    && match of {
                        None => true,
                        Some(Records::Key(selected)) => selected == key,
                        Some(Records::CreatedDflt(subkeys)) => record.is_some_and(|r| {
                            r.creator == node
                                && r.schema
                                    == (RecordSchema::Dflt {
                                        owner_subkeys: *subkeys,
                                    })
                        }),
                    }
            }
        }
    }
}

impl Faults {
    /// How many replicas take `node`'s set of `key`/`subkey`: `None` when
    /// no write fault selects it, else the fewest any selecting fault
    /// allows (`0` for [`Fault::WriteFails`]).
    pub(crate) fn accepting(
        &self,
        node: usize,
        key: &str,
        subkey: u32,
        record: Option<&NetRecord>,
    ) -> Option<usize> {
        self.write_limits
            .iter()
            .filter(|((n, of), _)| *n == node && of.selects(node, key, subkey, record))
            .map(|(_, accepting)| *accepting)
            .min()
    }
}

impl Netsim {
    /// Put `fault` in force.
    pub fn fault(&self, fault: Fault) {
        match fault {
            Fault::TryAgain { node, calls } => {
                self.net.inner.lock().faults.try_again.insert(node, calls);
            }
            Fault::DhtOffline { node, offline } => {
                toggle(&mut self.net.inner.lock().faults.dht_offline, node, offline);
            }
            Fault::WriteFails { node, of } => {
                self.net
                    .inner
                    .lock()
                    .faults
                    .write_limits
                    .insert((node, of), 0);
            }
            Fault::WritePartial {
                node,
                of,
                accepting,
            } => {
                assert!(
                    (1..GET_COUNT).contains(&accepting),
                    "a partial write is accepted by 0 < k < get_value_count replicas"
                );
                self.net
                    .inner
                    .lock()
                    .faults
                    .write_limits
                    .insert((node, of), accepting);
            }
            Fault::Partition { a, b } => {
                let mut inner = self.net.inner.lock();
                for x in &a {
                    for y in &b {
                        inner.faults.partitions.insert(pair(*x, *y));
                    }
                }
            }
            Fault::Heal => self.heal(),
            Fault::DropMessages { per_mille } => {
                self.net.inner.lock().faults.drop_per_mille = per_mille.min(1000);
            }
            Fault::DropFrom { node, per_mille } => {
                let drop_from = &mut self.net.inner.lock().faults.drop_from;
                if per_mille == 0 {
                    drop_from.remove(&node);
                } else {
                    drop_from.insert(node, per_mille.min(1000));
                }
            }
            Fault::Unanswered { node, unanswered } => {
                toggle(
                    &mut self.net.inner.lock().faults.unanswered,
                    node,
                    unanswered,
                );
            }
            Fault::CallHangs { node, hangs } => {
                toggle(&mut self.net.inner.lock().faults.hung, node, hangs);
            }
            Fault::SendHangs { node, hangs } => {
                toggle(&mut self.net.inner.lock().faults.send_hung, node, hangs);
            }
            Fault::ClockSkew { node, ms } => {
                if let Some(clock) = self.clocks.lock().get(node) {
                    clock.set_skew_ms(ms);
                }
                if let Some(slot) = self.net.inner.lock().nodes.get_mut(node) {
                    slot.skew_ms = ms;
                }
            }
            Fault::WatchDeath { node, key } => self.net.kill_watch(node, &key),
            Fault::RouteDeath { node } => self.kill_routes(node),
            Fault::RouteLinkFailure { node } => {
                self.net.inner.lock().faults.link_failure.insert(node);
            }
            Fault::LoopbackFailures { node, class } => self.net.break_route(node, class),
            Fault::UdpBlocked { node, blocked } => self.net.block_udp(node, blocked),
            Fault::Stall { node, ms } => self.stall(node, std::time::Duration::from_millis(ms)),
            Fault::Detach { node, detached } => self.detach(node, detached),
            Fault::InternetReady { node, ready } => self.internet_ready(node, ready),
            Fault::ReplicaChurn { key, count } => self.churn(&key, count),
            Fault::Sleep { node, asleep } => {
                toggle(&mut self.net.inner.lock().faults.asleep, node, asleep);
            }
        }
    }

    fn heal(&self) {
        let unready: Vec<usize> = {
            let mut inner = self.net.inner.lock();
            inner.faults.partitions.clear();
            inner.faults.dht_offline.clear();
            inner.faults.write_limits.clear();
            let mut nodes: Vec<usize> = inner.faults.not_ready.iter().copied().collect();
            nodes.sort_unstable();
            nodes
        };
        for node in unready {
            self.internet_ready(node, true);
        }
    }

    /// The node's routes die: it hears `dead_routes`; no importer hears
    /// (Veilid tells an importer only of its own cache, [`routes`]).
    fn kill_routes(&self, node: usize) {
        let dead: Vec<String> = {
            let mut inner = self.net.inner.lock();
            let dead: Vec<String> = inner
                .routes
                .iter()
                .filter(|(_, r)| r.node == node)
                .map(|(id, _)| id.clone())
                .collect();
            for id in &dead {
                inner.routes.remove(id);
            }
            // Veilid's own value-changed route dies with them.
            if let Some(slot) = inner.nodes.get_mut(node) {
                slot.route_epoch += 1;
            }
            dead
        };
        self.net.send(
            node,
            node,
            "RouteChange",
            PortEvent::RouteChange {
                dead,
                dead_remote: Vec::new(),
            },
            false,
        );
    }
}
