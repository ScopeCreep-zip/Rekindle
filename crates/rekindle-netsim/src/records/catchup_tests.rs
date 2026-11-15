//! The catch-up (M2) and offline open/get (M3) rules (plan E2.B S6), each
//! against the Veilid lines [`super::catchup`] and [`super::dht`] cite.

use std::sync::Arc;
use std::time::Duration;

use rekindle_records::port::{DhtBackend, DhtErrorKind, PortEvent, RecordSchema, ValueChange};

use super::catchup::MAX_IN_FLIGHT;
use super::dht::SimDht;
use super::outbound::WatchState;
use crate::{Fault, Netsim, Node};

fn dht(sim: &Netsim, node: &Node) -> SimDht {
    SimDht::new(node.index, Arc::clone(&sim.net), Arc::clone(&node.rng))
}

/// A record node 0 created and seeded at subkeys 0..4, read in full and
/// watched by node 1 (so its establishing inspection finds nothing).
async fn watched(sim: &Netsim, nodes: &mut [Node]) -> (String, SimDht, SimDht) {
    let (a, b) = (dht(sim, &nodes[0]), dht(sim, &nodes[1]));
    let key = a
        .create(&RecordSchema::Dflt { owner_subkeys: 4 }, None)
        .await
        .unwrap()
        .key;
    for subkey in 0..4 {
        a.set(&key, subkey, vec![0], None).await.unwrap();
    }
    b.open(&key, None, rekindle_records::port::OpenIntent::Hold)
        .await
        .unwrap();
    for subkey in 0..4 {
        b.get(&key, subkey, true).await.unwrap();
    }
    assert!(b.watch(&key, None, None, None).await.unwrap());
    assert!(heard(sim, nodes, 1).await.is_empty());
    (key, a, b)
}

/// Run a watch tick, wait out an inspection's latency, deliver; the value
/// changes node `i` heard.
async fn heard(sim: &Netsim, nodes: &mut [Node], i: usize) -> Vec<ValueChange> {
    sim.net.inner.lock().next_tick = Some(tokio::time::Instant::now());
    sim.net.deliver();
    tokio::time::advance(Duration::from_secs(7)).await;
    sim.net.deliver();
    let mut changes = Vec::new();
    while let Ok(event) = nodes[i].events.try_recv() {
        if let PortEvent::ValueChange(change) = event {
            changes.push(change);
        }
    }
    changes
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_reattached_node_hears_the_changes_it_missed() {
    let (sim, mut nodes) = Netsim::nodes(2, 21);
    let (key, a, b) = watched(&sim, &mut nodes).await;
    sim.fault(Fault::Detach {
        node: 1,
        detached: true,
    });
    a.set(&key, 1, vec![1], None).await.unwrap();
    a.set(&key, 2, vec![2], None).await.unwrap();
    assert!(heard(&sim, &mut nodes, 1).await.is_empty());
    sim.fault(Fault::Detach {
        node: 1,
        detached: false,
    });
    let changes = heard(&sim, &mut nodes, 1).await;
    // Several newer subkeys: the range with no value and no read
    // (0.5.8 `watch_value.rs:1492-1501`); the local copy stays.
    assert_eq!(changes.len(), 1, "{changes:?}");
    assert_eq!(changes[0].subkeys, vec![1, 2]);
    assert_eq!(changes[0].value, None);
    assert_eq!(b.get(&key, 1, false).await.unwrap().unwrap().seq, 0);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn dht_back_online_with_nothing_newer_reports_nothing() {
    let (sim, mut nodes) = Netsim::nodes(2, 22);
    let (_key, _a, _b) = watched(&sim, &mut nodes).await;
    for offline in [true, false] {
        sim.fault(Fault::DhtOffline { node: 1, offline });
        assert!(heard(&sim, &mut nodes, 1).await.is_empty());
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn heal_is_a_readiness_return_and_an_offline_node_holds_its_inspections() {
    let (sim, mut nodes) = Netsim::nodes(2, 23);
    let (key, a, _b) = watched(&sim, &mut nodes).await;
    sim.fault(Fault::DhtOffline {
        node: 1,
        offline: true,
    });
    a.set(&key, 3, vec![3], None).await.unwrap();
    sim.fault(Fault::Heal);
    let changes = heard(&sim, &mut nodes, 1).await;
    assert_eq!(changes.len(), 1, "{changes:?}");
    assert_eq!(changes[0].subkeys, vec![3]);
    // While offline the queue holds, unspent (0.5.8 `mod.rs:1129-1131`),
    // and no fallback fires.
    super::catchup::enqueue(&mut sim.net.inner.lock(), 1, &key, sim.net.now_us());
    let before = state(&sim, &key).last_fallback;
    let queued = sim.net.inner.lock().nodes[1]
        .outbound
        .queue
        .get(&key)
        .copied();
    sim.net.inner.lock().faults.dht_offline.insert(1);
    for _ in 0..400 {
        step(&sim, &mut nodes, 1).await;
    }
    assert_eq!(
        sim.net.inner.lock().nodes[1]
            .outbound
            .queue
            .get(&key)
            .copied(),
        queued
    );
    assert_eq!(state(&sim, &key).last_fallback, before);
    sim.net.inner.lock().faults.dht_offline.remove(&1);
    step(&sim, &mut nodes, 1).await;
    assert!(!sim.net.inner.lock().nodes[1]
        .outbound
        .queue
        .contains_key(&key));
}

fn state(sim: &Netsim, key: &str) -> WatchState {
    sim.net.inner.lock().nodes[1].outbound.states[key].clone()
}

/// One 1 s watch tick and its deliveries; what node `i` heard.
async fn step(sim: &Netsim, nodes: &mut [Node], i: usize) -> Vec<ValueChange> {
    sim.net.inner.lock().next_tick = Some(tokio::time::Instant::now());
    sim.net.deliver();
    tokio::time::advance(Duration::from_secs(1)).await;
    sim.net.deliver();
    let mut changes = Vec::new();
    while let Ok(event) = nodes[i].events.try_recv() {
        if let PortEvent::ValueChange(change) = event {
            changes.push(change);
        }
    }
    changes
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_fallback_backs_off_from_30_s_to_300_s_while_it_finds_nothing() {
    let (sim, mut nodes) = Netsim::nodes(2, 24);
    let (key, _a, _b) = watched(&sim, &mut nodes).await;
    let mut fired = Vec::new();
    for _ in 0..1_400 {
        assert!(step(&sim, &mut nodes, 1).await.is_empty());
        let last = state(&sim, &key).last_fallback;
        if last.is_some() && fired.last() != Some(&last) {
            fired.push(last);
        }
    }
    let gaps: Vec<u64> = fired
        .windows(2)
        .map(|w| (w[1].unwrap() - w[0].unwrap()) / 1_000_000)
        .collect();
    // The establishing inspection already backed 30 s off to 60 s
    // (`note_change_inspect_outcome`, `watch_value.rs:2368-2397`).
    assert_eq!(gaps[..4], [120, 240, 300, 300], "{gaps:?}");
    assert_eq!(state(&sim, &key).interval(), 300_000_000);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_push_lost_in_a_sleep_surfaces_at_the_fallback_on_the_wall_clock() {
    let (sim, mut nodes) = Netsim::nodes(2, 27);
    let (key, a, _b) = watched(&sim, &mut nodes).await;
    for _ in 0..200 {
        step(&sim, &mut nodes, 1).await;
    }
    sim.fault(Fault::Sleep {
        node: 1,
        asleep: true,
    });
    a.set(&key, 0, vec![9], None).await.unwrap();
    assert!(
        step(&sim, &mut nodes, 1).await.is_empty(),
        "the push is lost"
    );
    sim.fault(Fault::Sleep {
        node: 1,
        asleep: false,
    });
    // The wake: the wall clock jumped 10 min that the monotonic clock never
    // saw, so the silent watch is due at once.
    sim.fault(Fault::ClockSkew {
        node: 1,
        ms: 600_000,
    });
    let changes = step(&sim, &mut nodes, 1).await;
    let changes = if changes.is_empty() {
        heard(&sim, &mut nodes, 1).await
    } else {
        changes
    };
    assert_eq!(changes.len(), 1, "{changes:?}");
    assert_eq!(changes[0].subkeys, vec![0]);
    assert_eq!(
        changes[0].value.as_ref().map(|v| v.data.clone()),
        Some(vec![9])
    );
    // Productive: the backoff resets to 30 s.
    assert_eq!(state(&sim, &key).interval(), 30_000_000);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn at_most_16_inspections_run_at_once() {
    let (sim, mut nodes) = Netsim::nodes(2, 28);
    let (a, b) = (dht(&sim, &nodes[0]), dht(&sim, &nodes[1]));
    let mut keys = Vec::new();
    for _ in 0..20 {
        let key = a
            .create(&RecordSchema::Dflt { owner_subkeys: 1 }, None)
            .await
            .unwrap()
            .key;
        a.set(&key, 0, vec![0], None).await.unwrap();
        b.open(&key, None, rekindle_records::port::OpenIntent::Hold)
            .await
            .unwrap();
        assert!(b.watch(&key, None, None, None).await.unwrap());
        keys.push(key);
    }
    // Every watch was queued at its establishment; one tick starts 16.
    sim.net.inner.lock().next_tick = Some(tokio::time::Instant::now());
    sim.net.deliver();
    let inner = sim.net.inner.lock();
    assert_eq!(inner.nodes[1].outbound.in_flight.len(), MAX_IN_FLIGHT);
    assert_eq!(inner.nodes[1].outbound.queue.len(), 20 - MAX_IN_FLIGHT);
    drop(inner);
    let _ = &mut nodes;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_known_record_opens_and_reads_offline() {
    let (sim, mut nodes) = Netsim::nodes(2, 25);
    let (key, a, b) = watched(&sim, &mut nodes).await;
    b.close(&key).await.unwrap();
    let unknown = a
        .create(&RecordSchema::Dflt { owner_subkeys: 1 }, None)
        .await
        .unwrap()
        .key;
    a.set(&unknown, 0, vec![1], None).await.unwrap();
    sim.fault(Fault::DhtOffline {
        node: 1,
        offline: true,
    });
    // A record in the local store opens with no network (0.5.8 `open_record.rs:54-81`).
    b.open(&key, None, rekindle_records::port::OpenIntent::Hold)
        .await
        .unwrap();
    // A local get answers offline; a forced one is `TryAgain` (0.5.8 `get_value.rs:160-172`).
    assert_eq!(b.get(&key, 2, false).await.unwrap().unwrap().data, vec![0]);
    let forced = b.get(&key, 2, true).await.unwrap_err();
    assert_eq!(forced.kind, DhtErrorKind::TryAgain);
    // An unknown record needs the network (0.5.8 `open_record.rs:84-86`).
    let open = b
        .open(&unknown, None, rekindle_records::port::OpenIntent::Hold)
        .await
        .unwrap_err();
    assert_eq!(open.kind, DhtErrorKind::TryAgain);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_local_get_with_no_copy_is_try_again_offline() {
    let (sim, nodes) = Netsim::nodes(2, 26);
    let (a, b) = (dht(&sim, &nodes[0]), dht(&sim, &nodes[1]));
    let key = a
        .create(&RecordSchema::Dflt { owner_subkeys: 2 }, None)
        .await
        .unwrap()
        .key;
    a.set(&key, 0, vec![1], None).await.unwrap();
    b.open(&key, None, rekindle_records::port::OpenIntent::Hold)
        .await
        .unwrap();
    sim.fault(Fault::DhtOffline {
        node: 1,
        offline: true,
    });
    let err = b.get(&key, 0, false).await.unwrap_err();
    assert_eq!(err.kind, DhtErrorKind::TryAgain);
}
