//! Veilid's latency, always part of the model (plan E2.B). Every DHT call
//! through a session pool's backend (`records::dht::SimDht`) and every app message or call costs
//! simulated time drawn, per node, from a generator seeded by the run seed.
//! There is no switch that turns it off: a scenario's budgets are simulated
//! time on a paused runtime, so a real network's waits cost no wall time.
//!
//! # Calibration table
//!
//! Each duration is uniform over its [`Span`]. "Measured" numbers come from
//! the live first-login dev log of 2026-10-21 (veilid-core at the pinned fork
//! rev `0cd847de`, ADR 0017); "derived" numbers come from the pinned source
//! in `~/veilid/veilid-core/src` and have no measurement yet.
//!
//! | Call | Span | Source |
//! |------|------|--------|
//! | set (`set`, `set_durable`) | [`SET`] 9–18 s | measured: 11 serial prekey sets 18:19:01.290 → 18:21:29.194 = 13.4 s each; the creates' first sets 9–11 s apart (18:18:40 → :51, 18:21:29 → :38). Derived shape: consensus needs `set_value_count` 5 accepts of `set_value_fanout` 6 within `set_value_timeout_ms` 10 000 (`veilid_config.rs:913-915`), and the deadline stays a full timeout past the last accept (`rpc_processor/fanout/fanout_call.rs:642-656`) |
//! | one set RPC (lands on its replica) | [`SET_RPC`] 1–3 s | derived: the RPC path of a get's wave; the set's replicas take its value this far in, while the call itself returns at the end of its [`SET`] draw (`records/fanout.rs`) |
//! | get, inspect, open of a value the network holds | [`GET`] 1–3 s | derived: one fanout wave of `get_value_fanout` 5, consensus at `get_value_count` 3 (`veilid_config.rs:910-912`), each RPC far below `rpc.timeout_ms` 5 000 (`:852`) |
//! | get, inspect, open finding no value | [`DEADLINE`] 10.0–10.3 s | measured: the fresh prekey record's inspect 18:18:51.272 → 18:19:01.287 (`present=[]`); derived: no consensus is possible, so the fanout runs to `get_value_timeout_ms` 10 000 |
//! | watch | [`WATCH`] 1–3 s | derived: a watch is one fanout to the record's nodes (`storage_manager/watch_value.rs`), as a get |
//! | create | [`CREATE`] 5–20 ms | derived: `storage_manager/create_record.rs` makes the record locally with no fanout; the live ~10 s between creates is the first set, charged as a set |
//! | an app message's send returning | [`FIRST_HOP`] 40–120 ms | derived: one hop at the 80 ms median; the send waits for the first hop (`rpc_processor/mod.rs:1882-1926`) |
//! | app message, one way; an app call is two | [`MESSAGE`] 0.2–0.8 s | derived, not measured: our safety route (3–4 hops, `messages.rs` [`crate::call_timeout`]) plus the peer's 1-hop private route, at the 80 ms median hop latency the netsim reports in its attachment |
//!
//! Concurrency: calls on one node's `(record, subkey)` serialize, as
//! Veilid's per-subkey lock does (`storage_manager/set_value.rs:52`,
//! `record_lock_table`); calls on different subkeys overlap; at most
//! [`max_concurrent`] calls of a node run at once (the
//! `VeilidStartupOptions` default for `network.dht.max_concurrent_operations`)
//! and the excess queues. A subkey lock is taken before the cap, so a call
//! waiting on its subkey holds no slot.
//!
//! Faults compose: a node whose DHT is offline fails at once, as Veilid
//! checks `dht_is_online()` before any fanout (`set_value.rs:109`,
//! `open_record.rs:48`, `watch_value.rs:716`); a "try again" answer costs
//! the call's latency first, as a fanout that failed ran to its end.
//! A watch change is an RPC from the replica to the watcher: it arrives one
//! [`MESSAGE`] after the replica's tick sends it. Attachment events carry
//! no latency.
//!
//! # Recalibrating from a live log
//!
//! Run a dev build with `RUST_LOG=info`, unlock a fresh identity, and read
//! the gaps between `prekey record created`, `prekey maintenance`,
//! `profile record created` and `friend list record created`: the span
//! between maintenance and the profile, over the slot count written
//! serially, is one set; the record-created gaps are a first set each. Put
//! the new bounds in [`SET`] (and the others), with the log's timestamps in
//! the table above.

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

/// A set reaching consensus (see the table).
pub const SET: Span = Span::ms(9_000, 18_000);
/// One set RPC, from the node to a replica and its answer back: derived as
/// [`GET`]'s wave (the same RPC path), so a set's value is on its first
/// replicas 1–3 s in, as a reader sees it (`fanout_call.rs:630-656`).
pub const SET_RPC: Span = Span::ms(1_000, 3_000);
/// A get, inspect or open of a value the network holds.
pub const GET: Span = Span::ms(1_000, 3_000);
/// A get, inspect or open finding no value: the fanout's deadline.
pub const DEADLINE: Span = Span::ms(10_000, 10_300);
/// Establishing a watch.
pub const WATCH: Span = Span::ms(1_000, 3_000);
/// A local create.
pub const CREATE: Span = Span::ms(5, 20);
/// An app message, one way.
pub const MESSAGE: Span = Span::ms(200, 800);
/// An app message's send returning: one hop at the 80 ms median hop
/// latency the netsim reports (derived; `rpc_processor/mod.rs:1882-1926`
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
