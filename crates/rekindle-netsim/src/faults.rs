//! The faults a scenario puts in force (plan C9.3) and how the sim
//! applies them; split from `lib.rs` along its seam.

use rekindle_records::port::{RecordSchema, RouteClass};

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
    /// can; the writer hears `Ok(None)`, as [`Fault::WritePartial`]), until
    /// [`Fault::Heal`]; its other writes are unaffected.
    WriteFails { node: usize, of: Writes },
    /// Add latency before the selected writes reach their replicas.
    WriteStalls {
        node: usize,
        of: Writes,
        stall: std::time::Duration,
    },
    /// Every set by the node that `of` selects reaches only the
    /// `accepting` replicas closest to the record, which store it, until
    /// [`Fault::Heal`]. `0 < accepting < get_value_count`: Veilid counts a
    /// set that times out or exhausts with fewer than `get_value_count`
    /// `consensus_nodes` as offline (0.5.8 `storage_manager/mod.rs:990-1030`,
    /// `set_value.rs:711-738`), and one with more as landed, so only
    /// fewer is below consensus. With
    /// `AllowOffline(false)` the writer stores and queues nothing locally
    /// (`local_record_store_interface.rs:61-62`) and hears `Ok(None)`
    /// (fork `e1158844` `set_value.rs:731-738` then `:306-308`), while a
    /// reader whose fanout reaches an accepting replica sees the value.
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
    /// Every answer the node gives to an app call is lost on its way back
    /// (or no longer is): the question reaches the node, its handler runs
    /// and its `reply` succeeds, while the caller hears nothing and times
    /// out at its deadline. Veilid's app reply only completes the local
    /// waiter (fork `rpc_app_call.rs:160-170`); `answer` then renders the
    /// answer to the question's reply private route and hands it to the
    /// first hop (`rpc_processor/mod.rs:2328-2425`), and nothing tells the
    /// answerer of a loss past that hop; the caller's `wait_for_reply`
    /// ends in `Timeout` (`:708-729`), the callee having acted.
    AnswerLost { node: usize, lost: bool },
    /// A call to the node's routes never resolves: no answer and no
    /// Veilid `Timeout`, as a call stuck before Veilid's answer wait
    /// (route resolution, rendering, the first hop's send, none bounded:
    /// Veilid 0.5.8 `rpc_processor/mod.rs:2038-2104`; live finding 78). The question is
    /// lost; only the caller's own deadline ends it.
    CallHangs { node: usize, hangs: bool },
    /// A call to the node's routes is stuck for `stall` before it is sent
    /// (the same unbounded stages as [`Fault::CallHangs`]: route
    /// resolution, rendering, the first hop's send, Veilid 0.5.8
    /// `rpc_processor/mod.rs:2038-2104`), then goes as any call does; the
    /// answer wait starts once it is sent, as Veilid's does. A slow path,
    /// not a lost one. `None` ends it.
    CallStalls {
        node: usize,
        stall: Option<std::time::Duration>,
    },
    /// An app message to the node's routes is never dispatched: the send
    /// neither returns nor fails, as a send stuck in the stages before the
    /// first hop takes it (destination, render and safety-route compile,
    /// `send_envelope`, none bounded: Veilid 0.5.8 `rpc_processor/mod.rs:2196-2269`).
    /// The message is lost; only the sender's own deadline ends the send.
    SendHangs { node: usize, hangs: bool },
    /// The node's clock runs this far from simulated true time.
    ClockSkew { node: usize, ms: i64 },
    /// The node is a modified client: every set it makes of a subkey of
    /// `key` passes `SetDHTValueOptions::min_seqnum`, raising the seq to at
    /// least `min_seqnum` after the automatic increment (0.5.8
    /// `storage_manager/set_value.rs:136-142, 384-386`), as a writer may
    /// with the public API to skip seqs (plan E3.2 S9). `None` ends it; a
    /// client choice, not a network fault, so [`Fault::Heal`] keeps it.
    MinSeqnum {
        node: usize,
        key: String,
        min_seqnum: Option<u32>,
    },
    /// The next network get reaches only replicas at the currently stored
    /// value, even if the writer reaches other replicas before that get.
    NextGetFromLaggingReplica {
        node: usize,
        key: String,
        subkey: u32,
    },
    /// Drop value changes for one watcher and record, independently of app
    /// calls and of DHT reads and writes.
    DropValueChanges {
        node: usize,
        key: String,
        drop: bool,
    },
    /// The node's watch on `key` dies: Veilid sends a last change with
    /// empty subkeys, `count == 0` and no value (0.5.8 `watch_value.rs:782-789`).
    WatchDeath { node: usize, key: String },
    /// The node's own routes die: the owner hears of it, its importers are
    /// not told; each learns it from its own next remote route test
    /// (`dead_remote`, [`crate::loopback`]), and its sends are lost until
    /// then.
    RouteDeath { node: usize },
    /// The node's live own route of `class` stops delivering, and no one is
    /// told: the live case of finding 75. The route breaks as under
    /// [`Fault::LoopbackFailures`] while the node's online detector says
    /// Offline, so Veilid defers each failed loopback test instead of
    /// counting it (0.5.8 `routing_table/tasks/ping_validator.rs:815-826`),
    /// and never tells an importer. The route stays allocated and
    /// published: what is sent into it, or over it as a safety route, is
    /// lost (a call times out), and no `RouteChange` reaches the owner. It
    /// ends when the owner releases the route, or when a round trip proves
    /// the node's network (a returned readiness, a reattach, a replica's
    /// answer to a DHT question sent after the death) and the
    /// loopback failures, counted again, kill it ([`crate::loopback`]).
    RouteSilentDeath { node: usize, class: RouteClass },
    /// The node's live own route of `class` stops taking traffic from
    /// outside, while its loopback still passes (plan LF9, Linux r22): what
    /// enters it by the blob's entry hop, a peer's send or the node's own
    /// outside test, is lost (a call times out), while the node's loopback,
    /// the route as its own safety route, is turned around at the hop over
    /// the flow the node's own send just opened (veilid-core 0.5.8
    /// `route_compile.rs:103`, `:420-434`, `rpc_route.rs:329-365`) and passes.
    /// Veilid declares a route dead only on loopback failures or on lost
    /// questions with it as the safety route (`route_stats.rs:19-29`, `:117-131`),
    /// and charges a caller's lost question to the caller's safety route, so its
    /// owner is never told: the route stays allocated and published until its
    /// owner releases it. Each importer's own remote route test fails, and it
    /// hears `dead_remote` ([`crate::loopback`]).
    OutsideDead { node: usize, class: RouteClass },
    /// The importer's next remote route test of the owner's live route of
    /// `class` loses its answer, while the route lives and every call over
    /// it is answered: the live case of 2026-11-26 (diag RC6), a 3-hop
    /// round trip near Veilid's answer wait, where one status ping's answer
    /// came back past it. Veilid drops a late answer (0.5.8
    /// `operation_waiter.rs:179-193`), so the single test fails, and Veilid
    /// removes the import from that importer's cache and tells it alone
    /// `dead_remote` (`ping_validator.rs:741-744`, [`crate::loopback`]); the
    /// owner's loopback passes, so it is never told and never republishes.
    /// One test only: the importer's later tests of the route pass.
    RemoteTestLost {
        importer: usize,
        owner: usize,
        class: RouteClass,
    },
    /// The node's allocations of `class` fail their route test (or test
    /// again): Veilid allocates the route, its test over the network gets
    /// no answer, so it releases the route and answers `TryAgain`
    /// ("allocated route failed to test", veilid-core 0.5.8
    /// `veilid_api/api.rs:829-836`) once the test's duration has passed
    /// ([`routes`](crate::routes)); no route is left behind and nothing is
    /// published. The live F9 stage 2 case: a node whose routes cannot be
    /// proven keeps allocating. A client-side network condition, so
    /// [`Fault::Heal`] keeps it.
    AllocationTestFails {
        node: usize,
        class: RouteClass,
        failing: bool,
    },
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
    /// The node's automatic pool routes stop carrying traffic end to end
    /// (or carry it again), while its own allocated routes (General, Media,
    /// `Sending`) are unaffected: the live case of the 2026-11-19 Linux
    /// trace, where pool routes failed 3 to 5 loopback tests in a row
    /// before Veilid released them (0.5.8 `private_route_management.rs`,
    /// `route_spec_store/mod.rs:51-63`). What an unpinned send takes as its
    /// safety route is lost: a statement still "sent", since Veilid answers
    /// once the first hop takes it and counts no lost statement
    /// (`rpc_processor/mod.rs:2262-2320`, `route_stats.rs:144-147`, `:409`),
    /// and a question times out, its answer lost on a pool reply route too.
    /// The node's DHT questions ride the same pool (every record is opened
    /// `Safe`): its sets, gets, inspects, remote opens, watches,
    /// rehydrations and transactions reach no replica, and its watches'
    /// `ValueChanged` pushes, sent to their pool reply routes, are lost
    /// (`records::replicas::reaches` cites 0.5.8).
    PoolRoutesFail { node: usize, failing: bool },
    /// The node's own route of `class` stops carrying traffic end to end:
    /// what is sent over it or to it is lost, its loopback tests fail, and
    /// a run of 3 (Ordered) or 5 (Unordered) kills it, reported to the
    /// owner as `dead_routes` ([`crate::loopback`]).
    LoopbackFailures {
        node: usize,
        class: rekindle_records::port::RouteClass,
    },
    /// Veilid's evidence against the node's own route `id` crosses its
    /// threshold now (a run of failed loopback tests or lost questions,
    /// veilid-core 0.5.8 `route_stats.rs:19-29`, `:716-755`): Veilid releases
    /// that one route and tells the owner in `RouteChange.dead_routes`; the
    /// node's other routes, of its class too, are untouched.
    RouteDead { node: usize, id: String },
    /// The node's network blocks UDP (or stops blocking it): it cannot
    /// allocate an Unordered route (no fallback to Ordered,
    /// [`routes`](crate::routes)), and its live Unordered routes fail end
    /// to end as [`Fault::LoopbackFailures`] until UDP returns or they die.
    UdpBlocked { node: usize, blocked: bool },
    /// The node's Ordered (General) route loses every unordered send to it
    /// (or no longer does), while ordered sends and route tests to it, and
    /// unordered sends to its Unordered (Media) route, arrive. Veilid chooses
    /// and proves a private route's hops on that route's one ordering alone
    /// (0.5.8 fork `route_allocate.rs:580-588`, the pair probe `:897-936`)
    /// and tests it on that ordering (loopback `tasks/ping_validator.rs`
    /// `:673-676`, the remote test PreferOrdered `:692-697`). A routed
    /// operation carries the *sender's* sequencing and every hop filters the
    /// next hop by it (`rpc_processor/rpc_route.rs:26-29`, `:87-90`); Veilid
    /// adapts a preference only to the sender's own safety route
    /// (`route_compile.rs:106-110`, `safety.rs:33-38`
    /// `for_route_orderings`), never to the receiver's route. So a
    /// `PreferUnordered` send crosses UDP hop pairs nobody proved, where a
    /// 1.5–3 KB question goes as fragments with no retransmit
    /// (`veilid-tools/src/assembly_buffer.rs:15-21`): lost, a statement
    /// still "sent", a question timed out, the route never declared dead.
    /// Real-app DL1/DL2 (2026-11-25): unordered 1:1 calls to such routes
    /// arrived 8/48, ordered ones 119/122.
    UnprovenUnorderedHops { node: usize, unproven: bool },
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
    /// asleep its storage manager's tick does not run, nothing sent to it
    /// arrives (a value-changed push, an app message or call), its own DHT
    /// calls, sends and route allocations park until the wake and its wall
    /// clock stands still, then jumps across the sleep ([`crate::sleep`]);
    /// it stays attached, with no Offline transition. Veilid 0.5.8 has no
    /// tick-lag sweep (0.5.7's `CHANGE_INSPECT_LAG` is gone from
    /// `storage_manager/constants.rs`): a missed push surfaces through the
    /// fallback timer, on the node's wall clock
    /// (`records/outbound.rs`).
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
            Fault::WriteStalls { node, of, stall } => {
                self.net
                    .inner
                    .lock()
                    .faults
                    .write_stalls
                    .insert((node, of), stall);
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
            Fault::AnswerLost { node, lost } => {
                toggle(&mut self.net.inner.lock().faults.answer_lost, node, lost);
            }
            Fault::CallHangs { node, hangs } => {
                toggle(&mut self.net.inner.lock().faults.hung, node, hangs);
            }
            Fault::CallStalls { node, stall } => {
                let stalls = &mut self.net.inner.lock().faults.call_stall;
                match stall {
                    Some(stall) => stalls.insert(node, stall),
                    None => stalls.remove(&node),
                };
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
            Fault::MinSeqnum {
                node,
                key,
                min_seqnum,
            } => {
                let min_seqnums = &mut self.net.inner.lock().faults.min_seqnums;
                match min_seqnum {
                    Some(seq) => min_seqnums.insert((node, key), seq),
                    None => min_seqnums.remove(&(node, key)),
                };
            }
            Fault::NextGetFromLaggingReplica { node, key, subkey } => {
                let value = self
                    .network_get(&key, subkey)
                    .expect("establish the replica's value before arming the fault");
                self.net
                    .inner
                    .lock()
                    .faults
                    .lagging_gets
                    .insert((node, key, subkey), value);
            }
            Fault::DropValueChanges { node, key, drop } => {
                let changes = &mut self.net.inner.lock().faults.drop_value_changes;
                if drop {
                    changes.insert((node, key));
                } else {
                    changes.remove(&(node, key));
                }
            }
            Fault::WatchDeath { node, key } => self.net.kill_watch(node, &key),
            Fault::RouteDeath { node } => self.kill_routes(node),
            Fault::RouteSilentDeath { node, class } => self.net.silence_route(node, class),
            Fault::OutsideDead { node, class } => self.net.kill_outside(node, class),
            Fault::RemoteTestLost {
                importer,
                owner,
                class,
            } => self.net.lose_remote_test(importer, owner, class),
            Fault::AllocationTestFails {
                node,
                class,
                failing,
            } => {
                let set = &mut self.net.inner.lock().faults.allocation_test_fails;
                if failing {
                    set.insert((node, class));
                } else {
                    set.remove(&(node, class));
                }
            }
            Fault::RouteLinkFailure { node } => {
                self.net.inner.lock().faults.link_failure.insert(node);
            }
            Fault::PoolRoutesFail { node, failing } => {
                toggle(
                    &mut self.net.inner.lock().faults.pool_failing,
                    node,
                    failing,
                );
            }
            Fault::LoopbackFailures { node, class } => self.net.break_route(node, class),
            Fault::RouteDead { node, id } => self.net.kill_route(node, &id),
            Fault::UdpBlocked { node, blocked } => self.net.block_udp(node, blocked),
            Fault::UnprovenUnorderedHops { node, unproven } => {
                toggle(
                    &mut self.net.inner.lock().faults.unproven_unordered,
                    node,
                    unproven,
                );
            }
            Fault::Stall { node, ms } => self.stall(node, std::time::Duration::from_millis(ms)),
            Fault::Detach { node, detached } => self.detach(node, detached),
            Fault::InternetReady { node, ready } => self.internet_ready(node, ready),
            Fault::ReplicaChurn { key, count } => self.churn(&key, count),
            Fault::Sleep { node, asleep } => {
                if let Some(clock) = self.clocks.lock().get(node) {
                    clock.freeze(asleep);
                }
                toggle(&mut self.net.inner.lock().faults.asleep, node, asleep);
                self.net.sleep_changed();
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
