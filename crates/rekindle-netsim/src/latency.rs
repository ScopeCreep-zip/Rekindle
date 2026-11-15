//! Veilid's latency, always part of the model (plan E2.B). Every DHT call
//! through a session pool's backend (`records::dht::SimDht`) and every app message or call costs
//! simulated time drawn, per node, from a generator seeded by the run seed.
//! There is no switch that turns it off: a scenario's budgets are simulated
//! time on a paused runtime, so a real network's waits cost no wall time.
//!
//! # Calibration table
//!
//! Each duration is uniform over its [`Span`]. "Measured" numbers come from
//! the live A/B round 2 log of 2026-11-15
//! (`evidence/runs/2026-11-15-ab2-macA2-rekindled.log`, from 02:55:37;
//! veilid-core 0.5.8 at Rekindle's fork rev `91453e97`, ADR 0017);
//! "derived" numbers come from the 0.5.8 source in `veilid-core/src` and
//! have no measurement yet.
//!
//! | Call | Span | Source |
//! |------|------|--------|
//! | set reaching consensus (`set`, `set_durable`) | [`SET`] 0.33–1.42 s | measured: 26 presence status sets on the profile record, `publish_status: writing` → `published status to DHT` (e.g. 03:11:52.797 → :53.327, 03:12:52.797 → :53.230), 0.327–1.415 s, median 0.53 s; the mailbox create → its first set `Landed` 02:55:52.800 → :54.233 (1.43 s). Derived shape: 0.5.8 returns at `set_value_count` 5 accepts of `set_value_fanout` 6 (`veilid_config.rs:1011-1012`); the model's set ends at its fifth accept ([`crate::records`]' fanout), so [`SET`] states the measurement for budgets and the model draws [`SET_RPC`] |
//! | one set RPC (lands on its replica, its answer read) | [`SET_RPC`] 0.2–0.8 s | derived: the fifth of six lanes' answers over this span is 0.63 s on average and at most 0.8 s, as the measured [`SET`]; a set's replicas take its value this far in |
//! | a set that cannot reach consensus | its fanout's end | derived: exhausted at the last answer, a silent lane cut after max(1.65 s, 3 × the slowest answer) or timed out at `rpc.timeout_ms` 5 000, and `set_value_timeout_ms` 10 000 past the last accept (`fanout_call.rs:105-110,552-555`, `fanout_queue.rs:662-751`, `veilid_config.rs:944,1010`); no live sample (round 2 has no set below consensus) |
//! | get, inspect, open of a value the network holds | [`GET`] 0.40–0.65 s | measured: the poll's 255-subkey inspects of the held registry (`inspect_ms` 401, 497, 622, 399 at 03:12:53, 03:13:53, 03:14:53, 03:16:33) and the presence scan's inspect after its row write once a member row existed (03:12:51.711 → :52.182, :52.590 → :53.100, :53.546 → :53.980: 0.43–0.51 s) |
//! | get, inspect finding no value | [`DEADLINE`] 10.8–11.1 s | measured: the presence scan's 255-subkey inspect of a registry where no other row was written yet, from its row write to `read_changed … present=[]` (03:12:01.223 → :12.308, :17.813 → :28.724, :29.148 → :40.003, :40.415 → :51.229: 10.81–11.09 s); derived: an unwritten subkey has no consensus, so the fanout runs to `get_value_timeout_ms` 10 000 (`veilid_config.rs:1007`) |
//! | watch | [`WATCH`] 1–3 s | derived: a watch is one fanout to the record's nodes (`storage_manager/watch_value.rs`), as a get |
//! | create | [`CREATE`] 5–20 ms | derived: `storage_manager/create_record.rs` makes the record locally with no fanout; the live ~10 s between creates is the first set, charged as a set |
//! | an app message's send returning | [`FIRST_HOP`] 40–120 ms | derived: one hop at the 80 ms median; the send waits for the first hop (`rpc_processor/mod.rs:2262-2320`) |
//! | app message, one way; an app call is two | [`MESSAGE`] 0.2–0.8 s | derived, not measured: our safety route (3–4 hops, `messages.rs` [`crate::call_timeout`]) plus the peer's 1-hop private route, at the 80 ms median hop latency the netsim reports in its attachment |
//!
//! Concurrency: calls on one node's `(record, subkey)` serialize, as
//! Veilid's per-subkey lock does (`storage_manager/set_value.rs:176-183`,
//! `record_lock_table`); calls on different subkeys overlap; at most
//! [`max_concurrent`] calls of a node run at once (the
//! `VeilidStartupOptions` default for `network.dht.max_concurrent_operations`)
//! and the excess queues. A subkey lock is taken before the cap, so a call
//! waiting on its subkey holds no slot.
//!
//! Faults compose: a node whose DHT is offline fails a network call at
//! once, as Veilid checks `dht_is_online()` before its fanout (0.5.8
//! `set_value.rs:248`, `open_record.rs:86`). A watch call is not one:
//! `watch_values` (`watch_value.rs:100-120`) never checks online and records
//! the watch, and its reconcile skips while offline (`:1046-1049`), so an
//! offline watch only skips the latency. A "try again" answer costs the
//! call's latency first, as a fanout that failed ran to its end.
//! A watch change is an RPC from the replica to the watcher: it arrives one
//! [`MESSAGE`] after the replica's tick sends it. Attachment events carry
//! no latency.
//!
//! # Recalibrating from a live log
//!
//! Run `rekindled` with `RUST_LOG=debug` and read: a set from
//! `publish_status: writing to DHT` to `published status to DHT` (one set
//! of the held profile record); a get or inspect of held values from the
//! poll's `inspect_ms`; a no-value inspect from a presence scan's `presence
//! row written` to its next `read_changed … present=[]` while no other
//! member wrote. Put the new bounds in [`SET`] (and the others), with the
//! log's timestamps in the table above.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use tokio::sync::{OwnedMutexGuard, OwnedSemaphorePermit, Semaphore};

/// A uniform duration range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub min: Duration,
    pub max: Duration,
}

impl Span {
    const fn ms(min: u64, max: u64) -> Self {
        Self {
            min: Duration::from_millis(min),
            max: Duration::from_millis(max),
        }
    }

    /// The middle of the span.
    #[must_use]
    pub fn mean(&self) -> Duration {
        (self.min + self.max) / 2
    }
}

/// A set reaching consensus, as measured (see the table).
pub const SET: Span = Span::ms(330, 1_420);
/// One set RPC, from the node to a replica and its answer back (see the
/// table).
pub const SET_RPC: Span = Span::ms(200, 800);
/// A get, inspect or open of a value the network holds.
pub const GET: Span = Span::ms(400, 650);
/// A get or inspect finding no value: the fanout's deadline.
pub const DEADLINE: Span = Span::ms(10_800, 11_100);
/// Establishing a watch.
pub const WATCH: Span = Span::ms(1_000, 3_000);
/// A local create.
pub const CREATE: Span = Span::ms(5, 20);
/// An app message, one way.
pub const MESSAGE: Span = Span::ms(200, 800);
/// An app message's send returning: one hop at the 80 ms median hop
/// latency the netsim reports (derived; Veilid 0.5.8 `rpc_processor/mod.rs:2262-2320`
/// awaits the send to the first hop).
pub const FIRST_HOP: Span = Span::ms(40, 120);

/// At most this many DHT calls of one node run at once.
#[must_use]
pub fn max_concurrent() -> usize {
    let ops = rekindle_types::config::VeilidStartupOptions::default().max_concurrent_dht_operations;
    usize::try_from(ops).unwrap_or(usize::MAX).max(1)
}

/// A DHT call's place: its subkey locks and its slot under the cap, held
/// until the call's effect is applied.
pub(crate) struct Slot {
    _locks: Vec<OwnedMutexGuard<()>>,
    _permit: OwnedSemaphorePermit,
}

type LockKey = (usize, String, u32);

/// The latency model's state: per-node generators, subkey locks and caps.
pub(crate) struct Latency {
    seed: u64,
    rngs: Mutex<HashMap<usize, StdRng>>,
    locks: Mutex<HashMap<LockKey, Arc<tokio::sync::Mutex<()>>>>,
    caps: Mutex<HashMap<usize, Arc<Semaphore>>>,
}

impl Latency {
    pub(crate) fn new(seed: u64) -> Self {
        Self {
            seed,
            rngs: Mutex::new(HashMap::new()),
            locks: Mutex::new(HashMap::new()),
            caps: Mutex::new(HashMap::new()),
        }
    }

    /// A duration from `span`, drawn from `node`'s generator.
    pub(crate) fn draw(&self, node: usize, span: Span) -> Duration {
        let mut rngs = self.rngs.lock();
        let rng = rngs.entry(node).or_insert_with(|| {
            StdRng::seed_from_u64(self.seed ^ 0x6c61_7465_6e63_7900 ^ (node as u64).rotate_left(32))
        });
        let (lo, hi) = (span.min.as_micros(), span.max.as_micros());
        let micros = if hi > lo { rng.gen_range(lo..=hi) } else { lo };
        Duration::from_micros(u64::try_from(micros).unwrap_or(u64::MAX))
    }

    /// Wait for `subkeys` of `key` on `node`, then a slot under the cap,
    /// then the call's latency. The caller applies its effect while it
    /// holds the returned slot.
    pub(crate) async fn dht(&self, node: usize, span: Span, key: &str, subkeys: &[u32]) -> Slot {
        let slot = self.slot(node, key, subkeys).await;
        tokio::time::sleep(self.draw(node, span)).await;
        slot
    }

    /// Wait for `subkeys` of `key` on `node`, then a slot under the cap; the
    /// caller spends the call's latency itself.
    pub(crate) async fn slot(&self, node: usize, key: &str, subkeys: &[u32]) -> Slot {
        let mut sorted = subkeys.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        let mut locks = Vec::with_capacity(sorted.len());
        for subkey in sorted {
            let lock = Arc::clone(
                self.locks
                    .lock()
                    .entry((node, key.to_string(), subkey))
                    .or_default(),
            );
            locks.push(lock.lock_owned().await);
        }
        let cap = Arc::clone(
            self.caps
                .lock()
                .entry(node)
                .or_insert_with(|| Arc::new(Semaphore::new(max_concurrent()))),
        );
        let permit = cap
            .acquire_owned()
            .await
            .unwrap_or_else(|_| unreachable!("the cap's semaphore is never closed"));
        Slot {
            _locks: locks,
            _permit: permit,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::Instant;

    const FIXED: Span = Span::ms(1_000, 1_000);

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn one_subkey_serializes_and_two_overlap() {
        let model = Arc::new(Latency::new(1));
        let start = Instant::now();
        let run = |subkey: u32| {
            let model = Arc::clone(&model);
            tokio::spawn(async move {
                let _slot = model.dht(0, FIXED, "k", &[subkey]).await;
                Instant::now()
            })
        };
        let (a, b, c) = (run(0), run(0), run(1));
        let (a, b, c) = (a.await.unwrap(), b.await.unwrap(), c.await.unwrap());
        assert_eq!(a - start, Duration::from_secs(1));
        assert_eq!(b - start, Duration::from_secs(2));
        assert_eq!(c - start, Duration::from_secs(1));
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn the_cap_queues_the_excess() {
        let model = Arc::new(Latency::new(1));
        let cap = max_concurrent();
        let start = Instant::now();
        let tasks: Vec<_> = (0..=cap)
            .map(|i| {
                let model = Arc::clone(&model);
                let subkey = u32::try_from(i).unwrap();
                tokio::spawn(async move {
                    let _slot = model.dht(0, FIXED, "k", &[subkey]).await;
                    Instant::now()
                })
            })
            .collect();
        let mut done = Vec::new();
        for t in tasks {
            done.push(t.await.unwrap() - start);
        }
        let late = done
            .iter()
            .filter(|d| **d == Duration::from_secs(2))
            .count();
        assert_eq!(late, 1, "one call past the cap of {cap} waits a round");
    }

    #[test]
    fn a_seed_fixes_the_draws_within_the_span() {
        let draws = |seed| {
            let model = Latency::new(seed);
            (0..8).map(|_| model.draw(3, SET)).collect::<Vec<_>>()
        };
        assert_eq!(draws(5), draws(5));
        assert_ne!(draws(5), draws(6));
        assert!(draws(5).iter().all(|d| *d >= SET.min && *d <= SET.max));
    }
}
